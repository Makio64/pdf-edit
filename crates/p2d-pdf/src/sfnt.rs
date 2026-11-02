//! Small OpenType toolkit for direct editing.
//!
//! - [`web_font`] turns a font program embedded in a PDF into a font a browser accepts, so the
//!   editor can show a line being retyped in the PDF's own typeface. Embedded programs are often
//!   stripped to what a PDF reader needs (no `cmap`, `name`, `OS/2` or `post`, empty glyph slots):
//!   those tables are rebuilt, and the character map only lists glyphs that really have a shape.
//! - [`subset_truetype`] cuts an installed TrueType font down to the glyphs some new text needs,
//!   so it can be embedded in the PDF when the PDF's own font lacks a character.
//!
//! Outlines, advances and hinting are always copied byte for byte: glyphs keep their exact shape
//! and width. Kerning and ligature tables are left out on purpose, because a PDF draws glyphs one
//! by one at their own advance and the editor must lay text out the same way.

use std::collections::{BTreeMap, BTreeSet};

use p2d_ir::PathOp;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::types::Tag;
use skrifa::{FontRef, GlyphId, MetadataProvider};
#[cfg(test)]
use skrifa::raw::TableProvider;

/// Whether `data` is a font program a browser can load (TrueType or OpenType), not bare CFF.
pub fn is_sfnt(data: &[u8]) -> bool {
    matches!(data.get(..4), Some([0, 1, 0, 0] | b"true" | b"OTTO" | b"ttcf"))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    u16_at(b, at).map(|v| v as i16)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn table<'a>(font: &FontRef<'a>, tag: &[u8; 4]) -> Option<&'a [u8]> {
    font.table_data(Tag::new(tag)).map(|d| d.as_bytes())
}

/// Glyph offsets from `loca`, `num_glyphs + 1` entries.
fn loca_offsets(loca: &[u8], long: bool, num_glyphs: usize) -> Option<Vec<u32>> {
    (0..=num_glyphs)
        .map(|i| {
            if long {
                u32_at(loca, 4 * i)
            } else {
                u16_at(loca, 2 * i).map(|v| v as u32 * 2)
            }
        })
        .collect()
}

/// Counts drawing commands, to tell real glyphs from the empty slots a subset keeps.
struct CountPen(usize);

impl OutlinePen for CountPen {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn close(&mut self) {}
}

/// Whether `gid` draws something in `font`.
pub fn glyph_has_outline(font: &FontRef<'_>, gid: GlyphId) -> bool {
    let mut pen = CountPen(0);
    font.outline_glyphs().get(gid).is_some_and(|g| {
        g.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen)
            .is_ok()
            && pen.0 > 0
    })
}

/// Advance widths in font units, one per glyph (`hmtx` repeats its last advance).
fn advances(hhea: &[u8], hmtx: &[u8], num_glyphs: usize) -> Vec<u16> {
    let long = u16_at(hhea, 34).unwrap_or(0).max(1) as usize;
    let mut last = 0;
    (0..num_glyphs)
        .map(|g| {
            if g < long {
                last = u16_at(hmtx, 4 * g).unwrap_or(last);
            }
            last
        })
        .collect()
}

/// Checksum of a table as the sfnt directory defines it (sum of big-endian words, zero padded).
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |acc, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        acc.wrapping_add(u32::from_be_bytes(w))
    })
}

/// Assemble an sfnt file from its tables, with a correct directory and `head` checksum adjustment.
fn assemble(version: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| *tag);
    for (tag, data) in &mut tables {
        if tag == b"head" && data.len() >= 12 {
            data[8..12].copy_from_slice(&[0; 4]);
        }
    }
    let n = tables.len() as u16;
    let pow = 1u16 << (15 - n.max(1).leading_zeros() as u16);
    let mut out = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&(pow * 16).to_be_bytes());
    out.extend_from_slice(&(pow.trailing_zeros() as u16).to_be_bytes());
    out.extend_from_slice(&(n * 16 - pow * 16).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, data) in &tables {
        if tag == b"head" {
            head_at = Some(offset);
        }
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    if let Some(at) = head_at {
        let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at + 8..at + 12].copy_from_slice(&adjust.to_be_bytes());
    }
    out
}

/// `cmap` with a format 4 subtable (Unicode BMP) under platforms 0 and 3.
fn cmap_table(map: &BTreeMap<u32, u16>) -> Vec<u8> {
    // Segments of consecutive characters mapped to consecutive glyphs share one delta.
    let mut segs: Vec<(u16, u16, u16)> = Vec::new();
    for (&code, &gid) in map.iter().filter(|(c, _)| **c < 0xFFFF) {
        let code = code as u16;
        let delta = gid.wrapping_sub(code);
        match segs.last_mut() {
            Some((_, end, d)) if *end + 1 == code && *d == delta => *end = code,
            _ => segs.push((code, code, delta)),
        }
    }
    segs.truncate(8000);
    segs.push((0xFFFF, 0xFFFF, 1));
    let seg_count = segs.len() as u16;
    let pow = 1u16 << (15 - seg_count.leading_zeros() as u16);
    let mut sub = Vec::new();
    let len = 16 + 8 * segs.len();
    for v in [4, len as u16, 0, seg_count * 2, pow * 2, pow.trailing_zeros() as u16, seg_count * 2 - pow * 2] {
        sub.extend_from_slice(&v.to_be_bytes());
    }
    segs.iter().for_each(|s| sub.extend_from_slice(&s.1.to_be_bytes()));
    sub.extend_from_slice(&[0, 0]);
    segs.iter().for_each(|s| sub.extend_from_slice(&s.0.to_be_bytes()));
    segs.iter().for_each(|s| sub.extend_from_slice(&s.2.to_be_bytes()));
    segs.iter().for_each(|_| sub.extend_from_slice(&[0, 0]));

    let mut out = Vec::new();
    for v in [0u16, 2] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for (platform, encoding) in [(0u16, 3u16), (3, 1)] {
        out.extend_from_slice(&platform.to_be_bytes());
        out.extend_from_slice(&encoding.to_be_bytes());
        out.extend_from_slice(&20u32.to_be_bytes());
    }
    out.extend_from_slice(&sub);
    out
}

/// `name` table with the Windows Unicode records browsers read.
fn name_table(family: &str, subfamily: &str, ps_name: &str) -> Vec<u8> {
    let full = format!("{family} {subfamily}");
    let records = [(1u16, family), (2, subfamily), (3, ps_name), (4, full.as_str()), (6, ps_name)];
    let mut strings = Vec::new();
    let mut out = Vec::new();
    for v in [0u16, records.len() as u16, 6 + 12 * records.len() as u16] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for (id, text) in records {
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3u16, 1, 0x0409, id, utf16.len() as u16, strings.len() as u16] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        strings.extend_from_slice(&utf16);
    }
    out.extend_from_slice(&strings);
    out
}

/// Vertical metrics and style shared by the rebuilt `OS/2` and `post` tables.
struct FaceInfo {
    upem: u16,
    ascender: i16,
    descender: i16,
    line_gap: i16,
    avg_width: i16,
    bold: bool,
    italic: bool,
    first_char: u16,
    last_char: u16,
}

/// `OS/2` version 4 (96 bytes) with typographic metrics taken from `hhea`.
fn os2_table(f: &FaceInfo) -> Vec<u8> {
    let u = f.upem as i32;
    let scaled = |v: i32| (v * u / 1000) as i16;
    let mut out = Vec::with_capacity(96);
    let put16 = |out: &mut Vec<u8>, v: i16| out.extend_from_slice(&v.to_be_bytes());
    put16(&mut out, 4); // version
    put16(&mut out, f.avg_width);
    put16(&mut out, if f.bold { 700 } else { 400 });
    put16(&mut out, 5); // usWidthClass: normal
    put16(&mut out, 0); // fsType: installable
    for v in [650, 600, 0, 75, 650, 600, 0, 350] {
        put16(&mut out, scaled(v)); // sub- and superscript sizes and offsets
    }
    put16(&mut out, scaled(50)); // strikeout size
    put16(&mut out, scaled(260)); // strikeout position
    put16(&mut out, 0); // sFamilyClass
    out.extend_from_slice(&[0; 10]); // panose
    out.extend_from_slice(&[0; 16]); // ulUnicodeRange1-4
    out.extend_from_slice(b"P2D ");
    let mut selection = 0x0080u16; // USE_TYPO_METRICS
    if f.italic {
        selection |= 0x0001;
    }
    if f.bold {
        selection |= 0x0020;
    }
    if !f.italic && !f.bold {
        selection |= 0x0040;
    }
    out.extend_from_slice(&selection.to_be_bytes());
    out.extend_from_slice(&f.first_char.to_be_bytes());
    out.extend_from_slice(&f.last_char.to_be_bytes());
    put16(&mut out, f.ascender);
    put16(&mut out, f.descender);
    put16(&mut out, f.line_gap);
    out.extend_from_slice(&(f.ascender.max(0) as u16).to_be_bytes());
    out.extend_from_slice(&(f.descender.unsigned_abs()).to_be_bytes());
    out.extend_from_slice(&1u32.to_be_bytes()); // code page: Latin 1
    out.extend_from_slice(&0u32.to_be_bytes());
    put16(&mut out, scaled(500)); // sxHeight
    put16(&mut out, scaled(700)); // sCapHeight
    out.extend_from_slice(&[0, 0, 0, 32, 0, 1]); // default char, break char, max context
    out
}

/// `post` version 3 (no glyph names).
fn post_table(f: &FaceInfo) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(&0x0003_0000u32.to_be_bytes());
    let angle: i32 = if f.italic { -12 << 16 } else { 0 };
    out.extend_from_slice(&angle.to_be_bytes());
    out.extend_from_slice(&(-(f.upem as i32) / 10).to_be_bytes()[2..]);
    out.extend_from_slice(&((f.upem as i32) / 20).to_be_bytes()[2..]);
    out.extend_from_slice(&[0; 20]);
    out
}

/// `hmtx` and `hhea` with the advance of some glyphs replaced (`widths`: glyph id, 1000 units per
/// em); every glyph gets a full metric so each can have its own advance.
fn with_advances(hhea: &[u8], hmtx: &[u8], num_glyphs: usize, upem: u16, widths: &[(u16, f32)]) -> (Vec<u8>, Vec<u8>, Vec<u16>) {
    let mut advance = advances(hhea, hmtx, num_glyphs);
    for &(gid, w) in widths {
        if let Some(a) = advance.get_mut(gid as usize) {
            *a = (w * upem as f32 / 1000.0).round().max(0.0) as u16;
        }
    }
    let long = (u16_at(hhea, 34).unwrap_or(0).max(1) as usize).min(num_glyphs.max(1));
    let mut new_hmtx = Vec::with_capacity(4 * num_glyphs);
    for (g, a) in advance.iter().enumerate() {
        let lsb = if g < long {
            i16_at(hmtx, 4 * g + 2)
        } else {
            i16_at(hmtx, 4 * long + 2 * (g - long))
        };
        new_hmtx.extend_from_slice(&a.to_be_bytes());
        new_hmtx.extend_from_slice(&lsb.unwrap_or(0).to_be_bytes());
    }
    let mut new_hhea = hhea.to_vec();
    if new_hhea.len() >= 36 {
        let max = advance.iter().copied().max().unwrap_or(0);
        new_hhea[10..12].copy_from_slice(&max.to_be_bytes());
        new_hhea[34..36].copy_from_slice(&(num_glyphs as u16).to_be_bytes());
    }
    (new_hhea, new_hmtx, advance)
}

/// A browser font from an embedded `program` (TrueType or OpenType CFF), named `family`.
///
/// `chars` are the characters the PDF draws with this font and the glyph each one uses; the
/// program's own character map is added for glyphs that really have a shape. Characters whose
/// glyph is empty are left out, so the browser draws them with the next font of the CSS stack.
/// `widths` are the advances the PDF sets (glyph id, 1000 units per em): they replace the
/// program's, so the browser lays text out exactly as the PDF does. `None` when the program cannot
/// be read or nothing would be drawable.
pub fn web_font(
    program: &[u8],
    chars: &[(char, u16)],
    widths: &[(u16, f32)],
    family: &str,
    bold: bool,
    italic: bool,
) -> Option<Vec<u8>> {
    let font = FontRef::from_index(program, 0).ok()?;
    let head = table(&font, b"head")?;
    let hhea = table(&font, b"hhea")?;
    let hmtx = table(&font, b"hmtx")?;
    let maxp = table(&font, b"maxp")?;
    let num_glyphs = u16_at(maxp, 4)? as usize;
    let upem = u16_at(head, 18)?.max(16);
    let (new_hhea, new_hmtx, widths) = with_advances(hhea, hmtx, num_glyphs, upem, widths);
    let cff = table(&font, b"CFF ");
    let glyf_offsets = match cff {
        Some(_) => None,
        None => {
            let long = i16_at(head, 50)? != 0;
            let offsets = loca_offsets(table(&font, b"loca")?, long, num_glyphs)?;
            let glyf_len = table(&font, b"glyf")?.len() as u32;
            if offsets.windows(2).any(|w| w[0] > w[1]) || offsets.last().is_some_and(|&e| e > glyf_len) {
                return None;
            }
            Some(offsets)
        }
    };
    let drawable = |gid: u16| -> bool {
        let g = gid as usize;
        if g == 0 || g >= num_glyphs {
            return false;
        }
        match &glyf_offsets {
            Some(o) => o[g + 1] > o[g],
            None => glyph_has_outline(&font, GlyphId::new(gid as u32)),
        }
    };

    let mut map: BTreeMap<u32, u16> = BTreeMap::new();
    let add = |c: char, gid: u16, map: &mut BTreeMap<u32, u16>| {
        let ok = if c.is_whitespace() {
            (gid as usize) < num_glyphs && widths[gid as usize] > 0
        } else {
            drawable(gid)
        };
        if ok {
            map.entry(c as u32).or_insert(gid);
        }
    };
    for &(c, gid) in chars {
        add(c, gid, &mut map);
    }
    for (code, gid) in font.charmap().mappings() {
        if let (Some(c), Ok(gid)) = (char::from_u32(code), u16::try_from(gid.to_u32())) {
            add(c, gid, &mut map);
        }
    }
    // A no-break space draws like a space.
    if let Some(&space) = map.get(&0x20) {
        map.entry(0xA0).or_insert(space);
    }
    if !map.keys().any(|&c| !char::from_u32(c).is_some_and(char::is_whitespace)) {
        return None;
    }

    let mut used: Vec<u16> = map.values().map(|&g| widths[g as usize]).filter(|&w| w > 0).collect();
    used.sort_unstable();
    let info = FaceInfo {
        upem,
        ascender: i16_at(hhea, 4)?,
        descender: i16_at(hhea, 6)?,
        line_gap: i16_at(hhea, 8)?,
        avg_width: used.get(used.len() / 2).copied().unwrap_or(upem / 2) as i16,
        bold,
        italic,
        first_char: map.keys().next().map_or(0x20, |&c| c.min(0xFFFF) as u16),
        last_char: map.keys().next_back().map_or(0x20, |&c| c.min(0xFFFF) as u16),
    };
    let ps_name: String = family.chars().filter(char::is_ascii_alphanumeric).take(60).collect();
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"cmap", cmap_table(&map)),
        (*b"name", name_table(family, style_name(bold, italic), &ps_name)),
        (*b"OS/2", os2_table(&info)),
        (*b"post", post_table(&info)),
    ];
    let mut head = head.to_vec();
    if head.len() >= 46 {
        let style = u16::from(bold) | (u16::from(italic) << 1);
        head[44..46].copy_from_slice(&style.to_be_bytes());
    }
    tables.push((*b"head", head));
    tables.push((*b"hhea", new_hhea));
    tables.push((*b"hmtx", new_hmtx));
    let copied: &[&[u8; 4]] = if cff.is_some() {
        &[b"maxp", b"CFF "]
    } else {
        &[b"maxp", b"loca", b"glyf", b"cvt ", b"fpgm", b"prep", b"gasp"]
    };
    for tag in copied {
        if let Some(data) = table(&font, tag) {
            tables.push((**tag, data.to_vec()));
        }
    }
    let version = if cff.is_some() { u32::from_be_bytes(*b"OTTO") } else { 0x0001_0000 };
    Some(assemble(version, tables))
}

/// One glyph of [`outline_font`], as `glyf` data.
struct OutlineGlyph {
    data: Vec<u8>,
    advance: u16,
    /// `[x_min, y_min, x_max, y_max]`.
    bbox: [i16; 4],
    points: usize,
    contours: usize,
}

/// TrueType `glyf` data for an outline in em units (cubic curves turned into quadratic ones).
fn outline_glyph(path: &[PathOp], advance: f32) -> OutlineGlyph {
    use kurbo::{CubicBez, Point};
    let mut contours: Vec<Vec<(f64, f64, bool)>> = Vec::new();
    let mut cur: Vec<(f64, f64, bool)> = Vec::new();
    let mut last = (0.0f64, 0.0f64);
    // Start of the current subpath: after a close, drawing goes on from there.
    let mut start = (0.0f64, 0.0f64);
    let mut reopen = false;
    let close = |cur: &mut Vec<(f64, f64, bool)>, contours: &mut Vec<Vec<(f64, f64, bool)>>| {
        if cur.len() > 1 && cur.first() == cur.last() {
            cur.pop();
        }
        if cur.len() > 1 {
            contours.push(std::mem::take(cur));
        }
        cur.clear();
    };
    // Coordinates kept within what a glyph can hold, which also bounds how many quadratics a
    // cubic becomes.
    let f = |v: f32| if v.is_finite() { (v as f64).clamp(-16000.0, 16000.0) } else { 0.0 };
    for op in path {
        if !matches!(op, PathOp::MoveTo(_) | PathOp::Close) {
            if reopen && cur.is_empty() {
                cur.push((start.0, start.1, true));
            }
            reopen = false;
        }
        match *op {
            PathOp::MoveTo([x, y]) => {
                close(&mut cur, &mut contours);
                cur.push((f(x), f(y), true));
                last = (f(x), f(y));
                start = last;
                reopen = false;
            }
            PathOp::LineTo([x, y]) => {
                cur.push((f(x), f(y), true));
                last = (f(x), f(y));
            }
            PathOp::QuadTo([cx, cy, x, y]) => {
                cur.push((f(cx), f(cy), false));
                cur.push((f(x), f(y), true));
                last = (f(x), f(y));
            }
            PathOp::CurveTo([ax, ay, bx, by, x, y]) => {
                let cubic = CubicBez::new(
                    Point::new(last.0, last.1),
                    Point::new(f(ax), f(ay)),
                    Point::new(f(bx), f(by)),
                    Point::new(f(x), f(y)),
                );
                for (_, _, q) in cubic.to_quads(0.5) {
                    cur.push((q.p1.x, q.p1.y, false));
                    cur.push((q.p2.x, q.p2.y, true));
                }
                last = (f(x), f(y));
            }
            PathOp::Close => {
                close(&mut cur, &mut contours);
                last = start;
                reopen = true;
            }
        }
    }
    close(&mut cur, &mut contours);

    let round = |v: f64| v.round().clamp(-16000.0, 16000.0) as i16;
    let contours: Vec<Vec<(i16, i16, bool)>> = contours
        .into_iter()
        .map(|c| {
            let mut out: Vec<(i16, i16, bool)> = Vec::with_capacity(c.len());
            for (x, y, on) in c {
                let p = (round(x), round(y), on);
                if out.last() != Some(&p) {
                    out.push(p);
                }
            }
            out
        })
        .filter(|c| c.len() > 1)
        .collect();
    let advance = advance.round().clamp(0.0, 65535.0) as u16;
    let points: usize = contours.iter().map(Vec::len).sum();
    // More points or contours than a glyph can number: leave it blank.
    if points == 0 || points > u16::MAX as usize || contours.len() > i16::MAX as usize {
        return OutlineGlyph {
            data: Vec::new(),
            advance,
            bbox: [0; 4],
            points: 0,
            contours: 0,
        };
    }
    let all = || contours.iter().flatten();
    let bbox = [
        all().map(|p| p.0).min().unwrap_or(0),
        all().map(|p| p.1).min().unwrap_or(0),
        all().map(|p| p.0).max().unwrap_or(0),
        all().map(|p| p.1).max().unwrap_or(0),
    ];
    let mut data = Vec::new();
    data.extend_from_slice(&(contours.len() as i16).to_be_bytes());
    for v in bbox {
        data.extend_from_slice(&v.to_be_bytes());
    }
    let mut end = 0usize;
    for c in &contours {
        end += c.len();
        data.extend_from_slice(&((end - 1) as u16).to_be_bytes());
    }
    data.extend_from_slice(&0u16.to_be_bytes()); // no instructions
    data.extend(all().map(|p| u8::from(p.2)));
    for axis in 0..2 {
        let mut prev = 0i16;
        for p in all() {
            let v = if axis == 0 { p.0 } else { p.1 };
            data.extend_from_slice(&v.wrapping_sub(prev).to_be_bytes());
            prev = v;
        }
    }
    OutlineGlyph {
        data,
        advance,
        bbox,
        points,
        contours: contours.len(),
    }
}

/// A browser font drawn from glyph outlines in em space (1000 units per em), for PDF fonts whose
/// program a browser cannot load (Type 1, CFF, Type 3): each `(character, advance, outline)`
/// becomes a glyph mapped to its character.
pub fn outline_font(glyphs: &[(char, f32, &[PathOp])], family: &str, bold: bool, italic: bool) -> Option<Vec<u8>> {
    if glyphs.is_empty() || glyphs.len() >= u16::MAX as usize {
        return None;
    }
    let mut built: Vec<OutlineGlyph> = vec![outline_glyph(&[], 500.0)];
    let mut map: BTreeMap<u32, u16> = BTreeMap::new();
    for (c, advance, path) in glyphs {
        built.push(outline_glyph(path, *advance));
        map.insert(*c as u32, (built.len() - 1) as u16);
    }
    if let Some(&space) = map.get(&0x20) {
        map.entry(0xA0).or_insert(space);
    }
    let inked: Vec<&OutlineGlyph> = built.iter().filter(|g| g.points > 0).collect();
    let bound = |f: fn(&OutlineGlyph) -> i16, min: bool| {
        let it = inked.iter().map(|g| f(g));
        if min { it.min() } else { it.max() }.unwrap_or(0)
    };
    let (x_min, y_min, x_max, y_max) = (
        bound(|g| g.bbox[0], true),
        bound(|g| g.bbox[1], true),
        bound(|g| g.bbox[2], false),
        bound(|g| g.bbox[3], false),
    );
    let ascender = y_max.max(700);
    let descender = y_min.min(-200);

    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    let mut hmtx = Vec::new();
    for g in &built {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        glyf.extend_from_slice(&g.data);
        glyf.resize(glyf.len().div_ceil(4) * 4, 0);
        hmtx.extend_from_slice(&g.advance.to_be_bytes());
        hmtx.extend_from_slice(&g.bbox[0].to_be_bytes());
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());

    let n = built.len() as u16;
    let mut head = Vec::with_capacity(54);
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    head.extend_from_slice(&0u32.to_be_bytes());
    head.extend_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head.extend_from_slice(&0x000Bu16.to_be_bytes());
    head.extend_from_slice(&1000u16.to_be_bytes());
    head.extend_from_slice(&[0; 16]);
    for v in [x_min, y_min, x_max, y_max] {
        head.extend_from_slice(&v.to_be_bytes());
    }
    let style = u16::from(bold) | (u16::from(italic) << 1);
    for v in [style, 8, 2, 1, 0] {
        head.extend_from_slice(&v.to_be_bytes());
    }

    let advance_max = built.iter().map(|g| g.advance).max().unwrap_or(0);
    let min_rsb = inked.iter().map(|g| g.advance as i16 - g.bbox[2]).min().unwrap_or(0);
    let mut hhea = Vec::with_capacity(36);
    hhea.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for v in [ascender, descender, 0] {
        hhea.extend_from_slice(&v.to_be_bytes());
    }
    hhea.extend_from_slice(&advance_max.to_be_bytes());
    for v in [x_min, min_rsb, x_max, 1, 0, 0, 0, 0, 0, 0, 0] {
        hhea.extend_from_slice(&v.to_be_bytes());
    }
    hhea.extend_from_slice(&n.to_be_bytes());

    let mut maxp = Vec::with_capacity(32);
    maxp.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    let max_points = built.iter().map(|g| g.points).max().unwrap_or(0) as u16;
    let max_contours = built.iter().map(|g| g.contours).max().unwrap_or(0) as u16;
    for v in [n, max_points, max_contours, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        maxp.extend_from_slice(&v.to_be_bytes());
    }

    let mut advances: Vec<u16> = inked.iter().map(|g| g.advance).collect();
    advances.sort_unstable();
    let info = FaceInfo {
        upem: 1000,
        ascender,
        descender,
        line_gap: 0,
        avg_width: advances.get(advances.len() / 2).copied().unwrap_or(500) as i16,
        bold,
        italic,
        first_char: map.keys().next().map_or(0x20, |&c| c.min(0xFFFF) as u16),
        last_char: map.keys().next_back().map_or(0x20, |&c| c.min(0xFFFF) as u16),
    };
    let ps_name: String = family.chars().filter(char::is_ascii_alphanumeric).take(60).collect();
    let tables = vec![
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"maxp", maxp),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"glyf", glyf),
        (*b"cmap", cmap_table(&map)),
        (*b"name", name_table(family, style_name(bold, italic), &ps_name)),
        (*b"OS/2", os2_table(&info)),
        (*b"post", post_table(&info)),
    ];
    Some(assemble(0x0001_0000, tables))
}

fn style_name(bold: bool, italic: bool) -> &'static str {
    match (bold, italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "Bold Italic",
    }
}

/// An installed TrueType font cut down to some glyphs, ready to embed as a `CIDFontType2`.
pub struct TrueTypeSubset {
    /// The font program: every glyph keeps its id, glyphs that were not asked for are empty.
    pub program: Vec<u8>,
    pub postscript_name: String,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub cap_height: i16,
    pub bbox: [i16; 4],
    pub italic_angle: f32,
    /// Advance of each glyph, in font units.
    pub advances: Vec<u16>,
}

/// Glyph id of each character of `text` in the face `index` of `data`, `None` for characters the
/// font cannot draw (a whitespace character only needs an advance).
pub fn map_chars(data: &[u8], index: u32, text: &str) -> Vec<Option<u16>> {
    let Ok(font) = FontRef::from_index(data, index) else {
        return text.chars().map(|_| None).collect();
    };
    let charmap = font.charmap();
    text.chars()
        .map(|c| {
            let lookup = if c == '\u{202F}' || c == '\u{00A0}' { ' ' } else { c };
            let gid = charmap.map(c).or_else(|| charmap.map(lookup))?;
            let ok = gid.to_u32() != 0 && (c.is_whitespace() || glyph_has_outline(&font, gid));
            ok.then(|| u16::try_from(gid.to_u32()).ok()).flatten()
        })
        .collect()
}

/// Advance of `c` in the face `index` of `data`, in 1000 units per em.
pub fn advance_1000(data: &[u8], index: u32, c: char) -> Option<f32> {
    let font = FontRef::from_index(data, index).ok()?;
    let gid = font.charmap().map(c)?;
    let upem = table(&font, b"head").and_then(|h| u16_at(h, 18))?.max(16) as f32;
    let advance = font.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(gid)?;
    Some(advance * 1000.0 / upem)
}

/// Cut the TrueType face `index` of `data` down to `gids` (and the glyphs their composites use).
/// `None` for CFF-based fonts, or when the font's license forbids embedding.
pub fn subset_truetype(data: &[u8], index: u32, gids: &BTreeSet<u16>) -> Option<TrueTypeSubset> {
    let font = FontRef::from_index(data, index).ok()?;
    let head = table(&font, b"head")?;
    let hhea = table(&font, b"hhea")?;
    let hmtx = table(&font, b"hmtx")?;
    let maxp = table(&font, b"maxp")?;
    let glyf = table(&font, b"glyf")?;
    let os2 = table(&font, b"OS/2");
    // fsType: restricted license only when no less restrictive usage bit is set (0x0004 preview
    // & print, 0x0008 editable), 0x0200 bitmap embedding only.
    let fs_type = os2.and_then(|t| u16_at(t, 8)).unwrap_or(0);
    if fs_type & 0x000E == 0x0002 || fs_type & 0x0200 != 0 {
        return None;
    }
    let num_glyphs = u16_at(maxp, 4)? as usize;
    let long = i16_at(head, 50)? != 0;
    let offsets = loca_offsets(table(&font, b"loca")?, long, num_glyphs)?;
    let glyph_data = |g: usize| -> &[u8] {
        let (a, b) = (offsets[g] as usize, offsets[g + 1] as usize);
        glyf.get(a..b.max(a)).unwrap_or(&[])
    };

    // Composite glyphs draw other glyphs: keep those too.
    let mut keep: BTreeSet<u16> = gids.iter().copied().filter(|&g| (g as usize) < num_glyphs).collect();
    keep.insert(0);
    let mut todo: Vec<u16> = keep.iter().copied().collect();
    while let Some(g) = todo.pop() {
        let d = glyph_data(g as usize);
        if i16_at(d, 0).is_none_or(|n| n >= 0) {
            continue;
        }
        let mut at = 10;
        while let (Some(flags), Some(component)) = (u16_at(d, at), u16_at(d, at + 2)) {
            if (component as usize) < num_glyphs && keep.insert(component) {
                todo.push(component);
            }
            at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
            at += match flags {
                f if f & 0x0008 != 0 => 2,
                f if f & 0x0040 != 0 => 4,
                f if f & 0x0080 != 0 => 8,
                _ => 0,
            };
            if flags & 0x0020 == 0 {
                break;
            }
        }
    }

    let mut new_glyf = Vec::new();
    let mut new_loca = Vec::with_capacity(4 * (num_glyphs + 1));
    for g in 0..num_glyphs {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        if keep.contains(&(g as u16)) {
            new_glyf.extend_from_slice(glyph_data(g));
            new_glyf.resize(new_glyf.len().div_ceil(4) * 4, 0);
        }
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    let mut new_head = head.to_vec();
    new_head.get_mut(50..52)?.copy_from_slice(&1i16.to_be_bytes());

    let mut tables = vec![
        (*b"head", new_head),
        (*b"loca", new_loca),
        (*b"glyf", new_glyf),
    ];
    for tag in [b"hhea", b"hmtx", b"maxp", b"cvt ", b"fpgm", b"prep"] {
        if let Some(data) = table(&font, tag) {
            tables.push((*tag, data.to_vec()));
        }
    }
    let upem = u16_at(head, 18)?.max(16);
    let ascender = i16_at(hhea, 4)?;
    let postscript_name = font
        .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Font".to_string())
        .chars()
        .filter(|c| c.is_ascii_graphic() && !"[](){}<>/%#".contains(*c))
        .collect();
    let italic_angle = table(&font, b"post")
        .and_then(|p| u32_at(p, 4))
        .map_or(0.0, |v| v as i32 as f32 / 65536.0);
    Some(TrueTypeSubset {
        program: assemble(0x0001_0000, tables),
        postscript_name,
        units_per_em: upem,
        ascender,
        descender: i16_at(hhea, 6)?,
        cap_height: os2
            .filter(|t| u16_at(t, 0).is_some_and(|v| v >= 2))
            .and_then(|t| i16_at(t, 88))
            .unwrap_or((ascender as i32 * 7 / 10) as i16),
        bbox: [i16_at(head, 36)?, i16_at(head, 38)?, i16_at(head, 40)?, i16_at(head, 42)?],
        italic_angle,
        advances: advances(hhea, hmtx, num_glyphs),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TrueType font installed on every Mac, when the tests run on one.
    fn system_truetype() -> Option<Vec<u8>> {
        ["/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf"]
            .iter()
            .find_map(|p| std::fs::read(p).ok())
    }

    #[test]
    fn test_outline_font_draws_the_given_shapes() {
        // A square "A" (cubic sides) and an empty space.
        let square = [
            PathOp::MoveTo([100.0, 0.0]),
            PathOp::CurveTo([100.0, 200.0, 100.0, 500.0, 100.0, 700.0]),
            PathOp::LineTo([600.0, 700.0]),
            PathOp::LineTo([600.0, 0.0]),
            PathOp::Close,
        ];
        let font = outline_font(&[('A', 700.0, &square[..]), (' ', 250.0, &[][..])], "p2d-test", false, false)
            .expect("font");
        let font = FontRef::new(&font).unwrap();
        let charmap = font.charmap();
        let a = charmap.map('A').expect("A mapped");
        let space = charmap.map(' ').expect("space mapped");
        assert!(glyph_has_outline(&font, a));
        assert!(!glyph_has_outline(&font, space));
        let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        assert_eq!(metrics.advance_width(a), Some(700.0));
        assert_eq!(metrics.advance_width(space), Some(250.0));
        let bounds = metrics.bounds(a).expect("bounds");
        assert_eq!((bounds.x_min, bounds.y_max), (100.0, 700.0));
    }

    #[test]
    fn test_outline_glyph_continues_from_a_closed_subpath() {
        // `h` then `l`: the second contour starts again at the first one's start.
        let path = [
            PathOp::MoveTo([0.0, 0.0]),
            PathOp::LineTo([500.0, 0.0]),
            PathOp::LineTo([500.0, 300.0]),
            PathOp::Close,
            PathOp::LineTo([0.0, 700.0]),
            PathOp::LineTo([250.0, 900.0]),
        ];
        let g = outline_glyph(&path, 600.0);
        assert_eq!((g.contours, g.points), (2, 6));
    }

    #[test]
    fn test_outline_glyph_bounds_huge_coordinates() {
        let path = [
            PathOp::MoveTo([0.0, 0.0]),
            PathOp::CurveTo([0.0, 1e30, 500.0, f32::INFINITY, 700.0, 0.0]),
            PathOp::Close,
        ];
        let g = outline_glyph(&path, 600.0);
        assert!(g.points > 0 && g.points < 200, "{} points", g.points);
        assert!(g.bbox.iter().all(|v| v.abs() <= 16000));
    }

    #[test]
    fn test_cmap_round_trip() {
        let map: BTreeMap<u32, u16> = [(0x41, 36), (0x42, 37), (0x43, 38), (0xE9, 120), (0x20AC, 3)].into();
        let cmap = cmap_table(&map);
        // Wrap the cmap alone in an sfnt so skrifa can read it back.
        let font = assemble(0x0001_0000, vec![(*b"cmap", cmap)]);
        let font = FontRef::new(&font).unwrap();
        let charmap = font.charmap();
        for (&c, &g) in &map {
            assert_eq!(charmap.map(c).map(|g| g.to_u32()), Some(g as u32), "U+{c:04X}");
        }
        assert_eq!(charmap.map('Z'), None);
    }

    #[test]
    fn test_subset_then_web_font_keep_shapes_and_advances() {
        let Some(arial) = system_truetype() else {
            return;
        };
        let text = "Écrire 42 €";
        let gids: Vec<u16> = map_chars(&arial, 0, text).into_iter().map(Option::unwrap).collect();
        let subset = subset_truetype(&arial, 0, &gids.iter().copied().collect()).expect("subset");
        assert!(subset.program.len() < arial.len() / 4, "{} bytes", subset.program.len());
        let sub = FontRef::new(&subset.program).unwrap();
        let orig = FontRef::new(&arial).unwrap();
        let unused = orig.charmap().map('Z').unwrap();
        assert!(!glyph_has_outline(&sub, unused), "glyphs not asked for are emptied");
        for &g in &gids {
            let gid = GlyphId::new(g as u32);
            assert_eq!(glyph_has_outline(&sub, gid), glyph_has_outline(&orig, gid));
        }

        // The subset has no cmap: the web font rebuilds one from the (char, glyph) pairs.
        let chars: Vec<(char, u16)> = text.chars().zip(gids.iter().copied()).collect();
        let web = web_font(&subset.program, &chars, &[], "p2d-test", false, false).expect("web font");
        let web = FontRef::new(&web).unwrap();
        let charmap = web.charmap();
        let metrics = web.glyph_metrics(Size::unscaled(), LocationRef::default());
        let orig_metrics = orig.glyph_metrics(Size::unscaled(), LocationRef::default());
        for (c, g) in &chars {
            let gid = charmap.map(*c).expect("mapped");
            assert_eq!(gid.to_u32(), *g as u32);
            assert_eq!(metrics.advance_width(gid), orig_metrics.advance_width(gid));
        }
        assert_eq!(charmap.map('Z'), None, "empty glyphs are not mapped");

        // Advances the PDF sets win over the program's.
        let e = gids[1];
        let web = web_font(&subset.program, &chars, &[(e, 600.0)], "p2d-test", false, false).unwrap();
        let web = FontRef::new(&web).unwrap();
        let upem = web.head().unwrap().units_per_em() as f32;
        let adv = web.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(GlyphId::new(e as u32));
        assert_eq!(adv, Some((600.0 * upem / 1000.0).round()));
        assert!(web.table_data(Tag::new(b"OS/2")).is_some() && web.table_data(Tag::new(b"name")).is_some());
    }
}
