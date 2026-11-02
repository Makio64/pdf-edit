//! Fonts for replacement text.
//!
//! First choice is the PDF's own embedded font, so an edited line keeps its typeface: a small font
//! dictionary is derived from the original one, pointing at the same embedded font program (nothing
//! is duplicated) and only adding the widths, and for CID fonts the `ToUnicode` map, of the glyphs
//! the new text uses. Characters that font cannot draw (subsets only hold the glyphs the document
//! used) come from an installed font, embedded as a subset ([`embed_truetype`]).

use std::collections::{BTreeSet, HashMap};

use hayro_interpret::hayro_syntax::object::dict::keys::{
    CID_TO_GID_MAP, DESCENDANT_FONTS, ENCODING, FONT_DESC, FONT_FILE2, SUBTYPE,
};
use hayro_interpret::hayro_syntax::object::{Array, Dict, Name, Object, Stream};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use super::lexer::dict_entries;
use crate::tounicode::to_unicode_chars;
use super::to_win_ansi;
use super::writer::{IncrementalUpdate, patch_dict};
use crate::sfnt::{glyph_has_outline, map_chars, subset_truetype};

/// A font object able to draw some text, added to the update.
pub struct DerivedFont {
    /// Number of the new font dictionary object.
    pub object: u32,
    /// The text as character codes of that font, `bytes_per_char` bytes per character.
    pub encoded: Vec<u8>,
    pub bytes_per_char: usize,
    /// Advance of each character in glyph space (1000 units per em).
    pub widths: Vec<f32>,
}

/// What the document says about how a font is used: the characters it draws in it, and the code
/// the edited line itself drew each of its characters with (a character may have several codes,
/// the line's own is the safe one).
#[derive(Default)]
pub struct FontUse<'k> {
    pub known: &'k str,
    pub line_codes: HashMap<char, u32>,
}

/// Character → code of `font`: the edited line's own codes first, then its `ToUnicode` map.
fn char_codes(font: &Dict<'_>, usage: &FontUse<'_>) -> HashMap<char, u32> {
    let mut codes = to_unicode_inverse(font);
    codes.extend(usage.line_codes.iter().map(|(c, code)| (*c, *code)));
    codes
}

/// Which characters of `text` the page font `font` can draw (see [`derive_font`]).
pub fn coverage(font: &Dict<'_>, text: &str, usage: &FontUse<'_>) -> Vec<bool> {
    let none = || text.chars().map(|_| false).collect::<Vec<_>>();
    let truetype = match font.get::<Name<'_>>(SUBTYPE).as_ref().map(|n| n.as_ref()) {
        Some(b"Type0") => cid_truetype_program(font).map(|(program, _)| {
            FontRef::new(&program).map_or_else(
                |_| none(),
                |p| cid_glyphs(font, &p, text, usage).iter().map(Option::is_some).collect(),
            )
        }),
        Some(b"TrueType") => simple_truetype_program(font).map(|program| {
            FontRef::new(&program).map_or_else(
                |_| none(),
                |p| simple_truetype_codes(font, &p, text, usage).iter().map(Option::is_some).collect(),
            )
        }),
        _ => None,
    };
    // Glyphs the program does not show us may still be drawn with the codes the PDF uses.
    let reused: Vec<bool> = reused_codes(font, text, usage).iter().map(Option::is_some).collect();
    match truetype {
        Some(own) => own.iter().zip(reused).map(|(a, b)| *a || b).collect(),
        None => reused,
    }
}

/// Derive a font able to draw `text` from the page font `font`, adding its objects to `update`;
/// `None` unless every character is covered (see [`coverage`]).
pub fn derive_font(
    font: &Dict<'_>,
    text: &str,
    usage: &FontUse<'_>,
    update: &mut IncrementalUpdate<'_>,
) -> Option<DerivedFont> {
    let truetype = match font.get::<Name<'_>>(SUBTYPE)?.as_ref() {
        b"Type0" => derive_cid_truetype(font, text, usage, update),
        b"TrueType" => derive_simple_truetype(font, text, usage, update),
        _ => None,
    };
    truetype.or_else(|| reuse_font(font, text, usage, update))
}

/// Code (bytes) and advance (1000 units per em) the font `font` uses for each character of
/// `text`, read back from its `ToUnicode` map (or its `WinAnsiEncoding`), for characters it
/// certainly has a glyph for: a Type 3 glyph procedure, a standard font the reader supplies, or
/// a character the document already draws in this font (`known`).
fn reused_codes(font: &Dict<'_>, text: &str, usage: &FontUse<'_>) -> Vec<Option<(Vec<u8>, f32)>> {
    let known = usage.known;
    let subtype = font.get::<Name<'_>>(SUBTYPE);
    let subtype = subtype.as_ref().map(|n| n.as_ref()).unwrap_or(b"");
    let type0 = subtype == b"Type0";
    let type3 = subtype == b"Type3";
    let identity = font.get::<Name<'_>>(ENCODING).is_some_and(|n| n.as_ref().starts_with(b"Identity-H"));
    let win_ansi = font.get::<Name<'_>>(ENCODING).is_some_and(|n| n.as_ref() == b"WinAnsiEncoding");
    let descendant = type0
        .then(|| font.get::<Array<'_>>(DESCENDANT_FONTS)?.iter::<Dict<'_>>().next())
        .flatten();
    let embedded = {
        let desc = descendant.as_ref().unwrap_or(font).get::<Dict<'_>>(FONT_DESC);
        desc.is_some_and(|d| [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"].iter().any(|k| d.contains_key(*k)))
    };
    if type0 && (!identity || descendant.is_none()) {
        return text.chars().map(|_| None).collect();
    }
    let codes = char_codes(font, usage);
    let cid_widths = descendant.as_ref().map(declared_cid_widths).unwrap_or_default();
    let default_width = descendant.as_ref().and_then(|d| d.get::<f32>(b"DW")).unwrap_or(1000.0);
    let type3_scale = font
        .get::<Array<'_>>(b"FontMatrix")
        .and_then(|m| m.iter::<f32>().next())
        .map_or(1.0, |a| a.abs() * 1000.0);
    let type3_glyphs = if type3 { type3_glyph_names(font) } else { HashMap::new() };
    text.chars()
        .map(|c| {
            let lookup = if c == '\u{202F}' { '\u{00A0}' } else { c };
            let code = codes
                .get(&lookup)
                .or_else(|| (lookup == '\u{00A0}').then(|| codes.get(&' ')).flatten())
                .copied()
                .or_else(|| {
                    let b = to_win_ansi(&c.to_string())[0];
                    (win_ansi && !type0 && (b != b'?' || c == '?')).then_some(b as u32)
                })?;
            let present = if type3 {
                type3_glyphs.get(&code).is_some_and(|has| *has)
            } else if embedded || type0 {
                known.contains(lookup) || (lookup == '\u{00A0}' && known.contains(' '))
            } else {
                true
            };
            if !present {
                return None;
            }
            if type0 {
                let width = cid_widths.get(&code).copied().unwrap_or(default_width);
                Some(((code as u16).to_be_bytes().to_vec(), width))
            } else {
                let code = u8::try_from(code).ok()?;
                let width = declared_simple_width(font, code).map(|w| w * if type3 { type3_scale } else { 1.0 });
                Some((vec![code], width.unwrap_or(0.0)))
            }
        })
        .collect()
}

/// For each code of a Type 3 font, whether it draws a glyph of its own (a procedure other than
/// the one code 0 falls back to, with a width).
fn type3_glyph_names(font: &Dict<'_>) -> HashMap<u32, bool> {
    let mut out = HashMap::new();
    let Some(procs) = font.get::<Dict<'_>>(b"CharProcs") else {
        return out;
    };
    let Some(encoding) = font.get::<Dict<'_>>(ENCODING) else {
        return out;
    };
    let Some(differences) = encoding.get::<Array<'_>>(b"Differences") else {
        return out;
    };
    let mut code = 0u32;
    let mut notdef: Option<Vec<u8>> = None;
    for item in differences.iter::<Object<'_>>() {
        match item {
            Object::Number(n) => code = n.as_f32() as u32,
            Object::Name(name) => {
                let name = name.as_ref().to_vec();
                if code == 0 {
                    notdef = Some(name.clone());
                }
                let drawn = procs.contains_key(name.as_slice())
                    && notdef.as_ref().is_none_or(|n| *n != name || code == 0)
                    && name != b".notdef";
                out.insert(code, drawn);
                code += 1;
            }
            _ => {}
        }
    }
    out
}

/// The page font `font` itself, copied, drawing `text` with the codes the PDF already uses.
fn reuse_font(font: &Dict<'_>, text: &str, usage: &FontUse<'_>, update: &mut IncrementalUpdate<'_>) -> Option<DerivedFont> {
    let codes: Vec<(Vec<u8>, f32)> = reused_codes(font, text, usage).into_iter().collect::<Option<_>>()?;
    let bytes_per_char = codes.first().map_or(1, |(c, _)| c.len());
    let object = update.add_object(font.data().to_vec());
    Some(DerivedFont {
        object,
        widths: codes.iter().map(|(_, w)| *w).collect(),
        encoded: codes.into_iter().flat_map(|(c, _)| c).collect(),
        bytes_per_char,
    })
}

/// Glyph of each character of `text` in `program`, `None` where it is missing or empty.
fn glyphs_for(program: &FontRef<'_>, text: &str) -> Vec<Option<GlyphId>> {
    let charmap = program.charmap();
    text.chars()
        .map(|c| {
            let lookup = if c == '\u{202F}' { '\u{00A0}' } else { c };
            let gid = charmap
                .map(lookup)
                .or_else(|| (lookup == '\u{00A0}').then(|| charmap.map(' ')).flatten())?;
            let ok = gid.to_u32() != 0 && (c.is_whitespace() || glyph_has_outline(program, gid));
            ok.then_some(gid)
        })
        .collect()
}

/// Character → code map read back from a font's `ToUnicode` CMap: for subset fonts whose embedded
/// program lost its own `cmap`, these are exactly the characters the subset can draw. The lowest
/// code wins when several draw the same character.
fn to_unicode_inverse(font: &Dict<'_>) -> HashMap<char, u32> {
    let mut codes: Vec<(u32, char)> = to_unicode_chars(font).into_iter().collect();
    codes.sort_unstable();
    let mut map = HashMap::new();
    for (code, c) in codes {
        map.entry(c).or_insert(code);
    }
    map
}

/// Mac OS Roman characters for codes 0x80–0xFF.
const MAC_ROMAN_HIGH: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»…\u{00A0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{F8FF}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

fn mac_roman_code(c: char) -> Option<u8> {
    if c.is_ascii() {
        return Some(c as u8);
    }
    let c = if c == '\u{202F}' { '\u{00A0}' } else { c };
    MAC_ROMAN_HIGH.chars().position(|m| m == c).map(|i| 0x80 + i as u8)
}

/// Glyphs of a simple TrueType font using `WinAnsiEncoding`, looked up the way PDF readers do
/// (ISO 32000 §9.6.6.4): through its Unicode `cmap`, else its Mac Roman one with the character's
/// Mac Roman code, else its symbolic one with the WinAnsi code (also at 0xF000 + code).
fn simple_truetype_glyphs(program: &FontRef<'_>, text: &str) -> Vec<Option<GlyphId>> {
    let unicode = glyphs_for(program, text);
    let Ok(cmap) = program.cmap() else {
        return unicode;
    };
    let subtable = |platform: u16, encoding: u16| {
        cmap.encoding_records()
            .iter()
            .find(|r| r.platform_id() as u16 == platform && r.encoding_id() == encoding)
            .and_then(|r| r.subtable(cmap.offset_data()).ok())
    };
    let (mac, symbol) = (subtable(1, 0), subtable(3, 0));
    text.chars()
        .zip(unicode)
        .map(|(c, found)| {
            found.or_else(|| {
                let win = to_win_ansi(&c.to_string())[0] as u32;
                let gid = mac
                    .as_ref()
                    .zip(mac_roman_code(c))
                    .and_then(|(t, code)| t.map_codepoint(code as u32))
                    .or_else(|| symbol.as_ref().and_then(|t| t.map_codepoint(0xF000 + win).or_else(|| t.map_codepoint(win))))?;
                let ok = gid.to_u32() != 0 && (c.is_whitespace() || glyph_has_outline(program, gid));
                ok.then_some(gid)
            })
        })
        .collect()
}

/// Like [`glyphs_for`], but finding glyphs through the PDF's `ToUnicode` map (identity CID → GID).
fn glyphs_via_to_unicode(
    font: &Dict<'_>,
    program: &FontRef<'_>,
    text: &str,
    usage: &FontUse<'_>,
) -> Vec<Option<GlyphId>> {
    let map = char_codes(font, usage);
    text.chars()
        .map(|c| {
            let lookup = if c == '\u{202F}' { '\u{00A0}' } else { c };
            let code = map
                .get(&lookup)
                .or_else(|| (lookup == '\u{00A0}').then(|| map.get(&' ')).flatten())?;
            let gid = GlyphId::new(*code);
            (c.is_whitespace() || glyph_has_outline(program, gid)).then_some(gid)
        })
        .collect()
}

/// Glyphs of a CID font: the ones the edited line drew, else through the program's own character
/// map, else the PDF's `ToUnicode`.
fn cid_glyphs(font: &Dict<'_>, program: &FontRef<'_>, text: &str, usage: &FontUse<'_>) -> Vec<Option<GlyphId>> {
    let own = glyphs_for(program, text);
    let via_pdf = glyphs_via_to_unicode(font, program, text, usage);
    text.chars()
        .zip(own.into_iter().zip(via_pdf))
        .map(|(c, (own, via_pdf))| if usage.line_codes.contains_key(&c) { via_pdf.or(own) } else { own.or(via_pdf) })
        .collect()
}

/// Widths a CID font declares for its glyphs (`/W`), by CID. Text set in a font derived from it
/// keeps them: producers may round them away from the program's metrics, and the rest of the
/// page is laid out with them.
fn declared_cid_widths(descendant: &Dict<'_>) -> HashMap<u32, f32> {
    let mut out = HashMap::new();
    let Some(w) = descendant.get::<Array<'_>>(b"W") else {
        return out;
    };
    let items: Vec<Object<'_>> = w.iter::<Object<'_>>().collect();
    let num = |o: &Object<'_>| match o {
        Object::Number(n) => Some(n.as_f32()),
        _ => None,
    };
    let mut i = 0;
    while let Some(first) = items.get(i).and_then(num) {
        let first = first as u32;
        match items.get(i + 1) {
            Some(Object::Array(ws)) => {
                for (k, w) in ws.iter::<f32>().enumerate() {
                    out.insert(first + k as u32, w);
                }
                i += 2;
            }
            Some(last) => {
                let (Some(last), Some(w)) = (num(last), items.get(i + 2).and_then(num)) else {
                    break;
                };
                for cid in first..=(last as u32).min(first + 0xFFFF) {
                    out.insert(cid, w);
                }
                i += 3;
            }
            None => break,
        }
    }
    out
}

/// Width a simple font declares for `code` (`/FirstChar`, `/Widths`), when it declares one.
fn declared_simple_width(font: &Dict<'_>, code: u8) -> Option<f32> {
    let first = font.get::<i32>(b"FirstChar")?;
    let widths = font.get::<Array<'_>>(b"Widths")?;
    let index = usize::try_from(code as i32 - first).ok()?;
    widths.iter::<f32>().nth(index).filter(|w| *w > 0.0)
}

/// Advance of `gid` in PDF glyph space (1000 units per em).
fn width_1000(program: &FontRef<'_>, gid: GlyphId) -> f32 {
    let upem = program.head().map(|h| h.units_per_em()).unwrap_or(1000).max(1) as f32;
    let metrics = program.glyph_metrics(Size::unscaled(), LocationRef::default());
    (metrics.advance_width(gid).unwrap_or(0.0) * 1000.0 / upem).round()
}

fn embedded_truetype(font_or_descendant: &Dict<'_>) -> Option<Vec<u8>> {
    let descriptor = font_or_descendant.get::<Dict<'_>>(FONT_DESC)?;
    let stream = descriptor.get::<Stream<'_>>(FONT_FILE2)?;
    stream.decoded().ok().map(|d| d.to_vec())
}

/// Raw bytes of the value of `key` in a dictionary.
fn raw_value(dict: &Dict<'_>, key: &[u8]) -> Option<Vec<u8>> {
    let data = dict.data();
    dict_entries(data)?
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, r)| data[r].to_vec())
}

fn stream_object(data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// Program of a `Type0` font with `Identity-H` encoding over a TrueType `CIDFontType2` with
/// identity CID→GID, and its descendant font.
fn cid_truetype_program<'a>(font: &Dict<'a>) -> Option<(Vec<u8>, Dict<'a>)> {
    if !font.get::<Name<'_>>(ENCODING)?.as_ref().starts_with(b"Identity-H") {
        return None;
    }
    let descendant = font.get::<Array<'_>>(DESCENDANT_FONTS)?.iter::<Dict<'_>>().next()?;
    if descendant.get::<Name<'_>>(SUBTYPE)?.as_ref() != b"CIDFontType2" {
        return None;
    }
    match descendant.get::<Name<'_>>(CID_TO_GID_MAP) {
        Some(map) if map.as_ref() != b"Identity" => return None,
        None if descendant.contains_key(CID_TO_GID_MAP) => return None,
        _ => {}
    }
    Some((embedded_truetype(&descendant)?, descendant))
}

/// Program of a simple TrueType font using `WinAnsiEncoding`.
/// Program of a simple embedded TrueType font.
fn simple_truetype_program(font: &Dict<'_>) -> Option<Vec<u8>> {
    embedded_truetype(font)
}

/// Code of each character of `text` in a simple TrueType font, with the glyph it draws; `None`
/// where the font cannot draw it. With `WinAnsiEncoding` the codes are WinAnsi's; otherwise
/// (symbolic subsets, whose codes are the producer's own) they are the codes the PDF itself uses
/// for each character, read back from `ToUnicode`, drawn through the font's symbolic or Mac
/// `cmap` as readers do.
fn simple_truetype_codes(
    font: &Dict<'_>,
    program: &FontRef<'_>,
    text: &str,
    usage: &FontUse<'_>,
) -> Vec<Option<(u8, GlyphId)>> {
    let win_ansi = font.get::<Name<'_>>(ENCODING).is_some_and(|n| n.as_ref() == b"WinAnsiEncoding");
    if win_ansi {
        return text
            .chars()
            .zip(simple_truetype_glyphs(program, text))
            .map(|(c, g)| {
                let code = to_win_ansi(&c.to_string())[0];
                g.filter(|_| code != b'?' || c == '?').map(|g| (code, g))
            })
            .collect();
    }
    let Ok(cmap) = program.cmap() else {
        return text.chars().map(|_| None).collect();
    };
    let subtables: Vec<_> = [(3u16, 0u16), (1, 0)]
        .iter()
        .filter_map(|&(platform, encoding)| {
            cmap.encoding_records()
                .iter()
                .find(|r| r.platform_id() as u16 == platform && r.encoding_id() == encoding)
                .and_then(|r| r.subtable(cmap.offset_data()).ok())
        })
        .collect();
    let codes = char_codes(font, usage);
    text.chars()
        .map(|c| {
            let lookup = if c == '\u{202F}' { '\u{00A0}' } else { c };
            let code = codes
                .get(&lookup)
                .or_else(|| (lookup == '\u{00A0}').then(|| codes.get(&' ')).flatten())
                .and_then(|&code| u8::try_from(code).ok())?;
            let gid = subtables.iter().find_map(|t| {
                t.map_codepoint(0xF000 + code as u32)
                    .or_else(|| t.map_codepoint(code as u32))
                    .filter(|g| g.to_u32() != 0)
            })?;
            (c.is_whitespace() || glyph_has_outline(program, gid)).then_some((code, gid))
        })
        .collect()
}

/// `Type0` font with `Identity-H` encoding over a TrueType `CIDFontType2` with identity CID→GID.
fn derive_cid_truetype(
    font: &Dict<'_>,
    text: &str,
    usage: &FontUse<'_>,
    update: &mut IncrementalUpdate<'_>,
) -> Option<DerivedFont> {
    let (program_bytes, descendant) = cid_truetype_program(font)?;
    let program = FontRef::new(&program_bytes).ok()?;
    let glyphs: Vec<(char, GlyphId)> = text
        .chars()
        .zip(cid_glyphs(font, &program, text, usage))
        .map(|(c, g)| g.map(|g| (c, g)))
        .collect::<Option<_>>()?;

    let declared = declared_cid_widths(&descendant);
    let width = |g: GlyphId| {
        declared
            .get(&g.to_u32())
            .copied()
            .unwrap_or_else(|| width_1000(&program, g))
    };
    let mut used: Vec<(GlyphId, char)> = glyphs.iter().map(|&(c, g)| (g, c)).collect();
    used.sort_by_key(|(g, _)| g.to_u32());
    used.dedup_by_key(|(g, _)| g.to_u32());

    let mut widths = String::from("[");
    let mut bfchars = String::new();
    for (gid, c) in &used {
        widths.push_str(&format!("{} [{}] ", gid.to_u32(), width(*gid)));
        let utf16: String = c.encode_utf16(&mut [0; 2]).iter().map(|u| format!("{u:04X}")).collect();
        bfchars.push_str(&format!("<{:04X}> <{utf16}>\n", gid.to_u32()));
    }
    widths.push(']');

    let new_descendant = patch_dict(descendant.data(), &[(b"W", widths.into_bytes())], &[])?;
    let descendant_num = update.add_object(new_descendant);
    let cmap = to_unicode_cmap(used.len(), &bfchars);
    let to_unicode_num = update.add_object(stream_object(cmap.as_bytes()));
    let base_font = raw_value(font, b"BaseFont").unwrap_or_else(|| b"/P2DFont".to_vec());
    let object = update.add_object(
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont {} /Encoding /Identity-H /DescendantFonts [{descendant_num} 0 R] /ToUnicode {to_unicode_num} 0 R >>",
            String::from_utf8_lossy(&base_font)
        )
        .into_bytes(),
    );
    let encoded = glyphs
        .iter()
        .flat_map(|(_, g)| (g.to_u32() as u16).to_be_bytes())
        .collect();
    Some(DerivedFont {
        object,
        encoded,
        bytes_per_char: 2,
        widths: glyphs.iter().map(|(_, g)| width(*g)).collect(),
    })
}

/// Simple TrueType font using `WinAnsiEncoding`: the derived copy gets widths for every code.
fn derive_simple_truetype(
    font: &Dict<'_>,
    text: &str,
    usage: &FontUse<'_>,
    update: &mut IncrementalUpdate<'_>,
) -> Option<DerivedFont> {
    let program_bytes = simple_truetype_program(font)?;
    let program = FontRef::new(&program_bytes).ok()?;
    let codes: Vec<(u8, GlyphId)> =
        simple_truetype_codes(font, &program, text, usage).into_iter().collect::<Option<_>>()?;
    let width = |code: u8, gid: GlyphId| declared_simple_width(font, code).unwrap_or_else(|| width_1000(&program, gid));
    // Every code the new text uses needs a width (subsets also use codes below 32): declare them
    // all, keeping the PDF's own.
    let mut all: Vec<f32> = (0u8..=255)
        .map(|code| declared_simple_width(font, code).unwrap_or(0.0))
        .collect();
    for &(code, gid) in &codes {
        all[code as usize] = width(code, gid);
    }
    let widths = format!("[{}]", all.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(" "));
    let derived = patch_dict(
        font.data(),
        &[
            (b"FirstChar", b"0".to_vec()),
            (b"LastChar", b"255".to_vec()),
            (b"Widths", widths.into_bytes()),
        ],
        &[],
    )?;
    let object = update.add_object(derived);
    Some(DerivedFont {
        object,
        widths: codes.iter().map(|&(code, gid)| width(code, gid)).collect(),
        encoded: codes.iter().map(|&(code, _)| code).collect(),
        bytes_per_char: 1,
    })
}

/// An installed font that can draw what the PDF's own font cannot.
#[derive(Clone)]
pub struct InstalledFace {
    /// The font file (a collection may hold several faces).
    pub data: p2d_ir::FontProgram,
    pub index: u32,
}

impl std::fmt::Debug for InstalledFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InstalledFace({:?}, face {})", self.data, self.index)
    }
}

/// Which characters of `text` the installed `face` can draw.
pub fn installed_coverage(face: &InstalledFace, text: &str) -> Vec<bool> {
    map_chars(&face.data.0, face.index, text).iter().map(Option::is_some).collect()
}

/// Embed the glyphs of `text` from the installed TrueType `face` as a `Type0` / `CIDFontType2`
/// font (identity CID→GID, so character codes are glyph ids). `None` when a character is missing,
/// the font is not TrueType, or its license forbids embedding.
pub fn embed_truetype(face: &InstalledFace, text: &str, update: &mut IncrementalUpdate<'_>) -> Option<DerivedFont> {
    let gids: Vec<u16> = map_chars(&face.data.0, face.index, text).into_iter().collect::<Option<_>>()?;
    let set: BTreeSet<u16> = gids.iter().copied().collect();
    let subset = subset_truetype(&face.data.0, face.index, &set)?;
    let scale = |v: i32| (v as f32 * 1000.0 / subset.units_per_em as f32).round();
    let width = |g: u16| scale(subset.advances.get(g as usize).copied().unwrap_or(0) as i32);

    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&subset.program, 6);
    let mut file = format!(
        "<< /Length {} /Length1 {} /Filter /FlateDecode >>\nstream\n",
        compressed.len(),
        subset.program.len()
    )
    .into_bytes();
    file.extend_from_slice(&compressed);
    file.extend_from_slice(b"\nendstream");
    let file_num = update.add_object(file);

    // A subset tag derived from the glyphs, as the PDF specification asks for embedded subsets.
    let tag: String = {
        let h = set.iter().fold(0x811C_9DC5u32, |h, &g| (h ^ g as u32).wrapping_mul(0x0100_0193));
        (0..6).map(|i| (b'A' + ((h >> (i * 5)) % 26) as u8) as char).collect()
    };
    let name = format!("{tag}+{}", subset.postscript_name);
    let [x0, y0, x1, y1] = subset.bbox.map(|v| scale(v as i32));
    let flags = if subset.italic_angle != 0.0 { 32 | 64 } else { 32 };
    let descriptor = update.add_object(
        format!(
            "<< /Type /FontDescriptor /FontName /{name} /Flags {flags} /FontBBox [{x0} {y0} {x1} {y1}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 {file_num} 0 R >>",
            subset.italic_angle,
            scale(subset.ascender as i32),
            scale(subset.descender as i32),
            scale(subset.cap_height as i32)
        )
        .into_bytes(),
    );
    let mut widths = String::from("[");
    let mut bfchars = String::new();
    let mut seen = BTreeSet::new();
    for (c, &g) in text.chars().zip(&gids) {
        if seen.insert(g) {
            widths.push_str(&format!("{g} [{}] ", width(g)));
            let utf16: String = c.encode_utf16(&mut [0; 2]).iter().map(|u| format!("{u:04X}")).collect();
            bfchars.push_str(&format!("<{g:04X}> <{utf16}>\n"));
        }
    }
    widths.push(']');
    let cid_font = update.add_object(
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor} 0 R /W {widths} /CIDToGIDMap /Identity >>"
        )
        .into_bytes(),
    );
    let cmap = to_unicode_cmap(seen.len(), &bfchars);
    let to_unicode = update.add_object(stream_object(cmap.as_bytes()));
    let object = update.add_object(
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [{cid_font} 0 R] /ToUnicode {to_unicode} 0 R >>"
        )
        .into_bytes(),
    );
    Some(DerivedFont {
        object,
        encoded: gids.iter().flat_map(|g| g.to_be_bytes()).collect(),
        bytes_per_char: 2,
        widths: gids.iter().map(|&g| width(g)).collect(),
    })
}

/// A `ToUnicode` CMap with `count` `bfchar` entries.
fn to_unicode_cmap(count: usize, bfchars: &str) -> String {
    format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /P2D-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n{count} beginbfchar\n{bfchars}endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
    )
}
