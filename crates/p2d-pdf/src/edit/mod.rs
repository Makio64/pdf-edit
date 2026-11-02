//! Direct PDF editing: replace text in place and save the result as an incremental update, so
//! everything that was not edited stays byte-for-byte (and so pixel-for-pixel) identical.
//!
//! The original glyphs of an edited line are switched to the invisible text mode right where they
//! are drawn (see [`content`]), which keeps every other glyph exactly in place without knowing any
//! glyph width. When that rewrite is not possible (text inside a form XObject, an encoding whose
//! character codes cannot be split, or a content stream that does not line up with the
//! extraction), the old text is hidden behind a box of the background colour instead. The new text
//! is set at the original size, colour, baseline, letter and word spacing and horizontal scale, in
//! the line's own embedded font for every character it has (see [`fonts`]). Other characters come
//! from the installed font closest to it, embedded as a subset, or as a last resort from the
//! closest standard PDF font (Helvetica, Times or Courier family), which every PDF reader has. A
//! right-aligned or centred line keeps its right edge or its middle.

mod content;
mod fonts;
pub mod lexer;
mod writer;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use hayro_interpret::hayro_syntax::Pdf;
use hayro_interpret::hayro_syntax::object::dict::keys::RESOURCES;
use hayro_interpret::hayro_syntax::page::Resources;
use hayro_interpret::hayro_syntax::object::{Dict, Name};
use hayro_interpret::util::TransformExt;
use kurbo::{Affine, Rect};
use p2d_ir::{Alignment, DocIr, RectPt, RgbaColor};

use content::Walker;
pub use fonts::InstalledFace;
use fonts::{DerivedFont, FontUse, coverage, derive_font, embed_truetype, installed_coverage};
use lexer::{fmt_num, write_hex_string};
use writer::{IncrementalUpdate, patch_dict, read_trailer};

/// Standard PDF font family used for replacement text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StandardFamily {
    Helvetica,
    Times,
    Courier,
}

impl StandardFamily {
    fn base_font(self, bold: bool, italic: bool) -> &'static str {
        match (self, bold, italic) {
            (Self::Helvetica, false, false) => "Helvetica",
            (Self::Helvetica, true, false) => "Helvetica-Bold",
            (Self::Helvetica, false, true) => "Helvetica-Oblique",
            (Self::Helvetica, true, true) => "Helvetica-BoldOblique",
            (Self::Times, false, false) => "Times-Roman",
            (Self::Times, true, false) => "Times-Bold",
            (Self::Times, false, true) => "Times-Italic",
            (Self::Times, true, true) => "Times-BoldItalic",
            (Self::Courier, false, false) => "Courier",
            (Self::Courier, true, false) => "Courier-Bold",
            (Self::Courier, false, true) => "Courier-Oblique",
            (Self::Courier, true, true) => "Courier-BoldOblique",
        }
    }
}

/// Replace the text drawn by some glyphs of a page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextEdit {
    pub page_index: usize,
    /// Indices into the page's extracted glyphs (`PageIr::glyphs`) that disappear.
    pub glyph_indices: Vec<usize>,
    /// Replacement text, in pieces of one style; empty to only delete.
    pub pieces: Vec<TextPiece>,
    /// Start of the replacement text: left edge and baseline, in visual page points.
    pub x_pt: f32,
    pub baseline_pt: f32,
    /// Right edge of the original line (visual page points), kept by right-aligned lines.
    pub x1_pt: f32,
    /// Which edge of the original line the new text keeps (left, right, or the middle).
    pub align: Alignment,
    /// Space after every character beyond its advance, in points (PDF `Tc`, visual size).
    pub char_spacing_pt: f32,
    /// Space after every space beyond `char_spacing_pt`, in points.
    pub word_spacing_pt: f32,
    /// Horizontal stretch of the glyphs (PDF `Tz` / 100); 0 or 1 for none.
    pub horiz_scale: f32,
    /// Area (visual page points) covered with `cover_color` when the old glyphs cannot be
    /// removed from the content stream.
    pub cover_bbox: RectPt,
    pub cover_color: RgbaColor,
}

impl TextEdit {
    /// The whole replacement text.
    pub fn text(&self) -> String {
        self.pieces.iter().map(|p| p.text.as_str()).collect()
    }
}

/// Part of the replacement text set in one style.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextPiece {
    pub text: String,
    /// A glyph of the original line in this style (index into `PageIr::glyphs`): its PDF font is
    /// used for every character it has.
    pub glyph_index: Option<usize>,
    /// Size the text appears at, used with installed and standard fonts.
    pub font_size_pt: f32,
    /// Size to use with the PDF's own font (the size the PDF itself sets).
    pub nominal_font_size_pt: f32,
    pub color: RgbaColor,
    pub family: StandardFamily,
    pub bold: bool,
    pub italic: bool,
    /// Installed font for characters the PDF's font cannot draw, set at `font_size_pt`.
    #[serde(skip)]
    pub fallback: Option<InstalledFace>,
    /// `fallback` is the PDF font's own family (same design): a piece needing it is then set in
    /// it entirely, rather than mixing two renderings of one typeface.
    #[serde(default)]
    pub fallback_same_family: bool,
    /// Characters the document draws in the PDF's font: whatever its kind, it can draw them again.
    #[serde(default)]
    pub known_chars: String,
    /// Other PDF fonts of the same typeface (subsets producers split it into), tried in turn for
    /// characters the first lacks: a glyph drawn in each, and the characters it draws.
    #[serde(default)]
    pub other_fonts: Vec<(usize, String)>,
    /// Advance of characters (1000 units per em) for PDF fonts that declare none, such as the
    /// standard fonts a PDF does not embed.
    #[serde(default)]
    pub char_widths: Vec<(char, f32)>,
}

/// A font of the PDF the new text may reuse: its dictionary, what the document shows of its use,
/// and how the content stream sets it (size, direction).
struct OwnFont<'a, 'k> {
    dict: Dict<'a>,
    usage: FontUse<'k>,
    setting: Option<(f64, [f64; 4])>,
}

/// A stretch of the new text drawn with one font.
struct Run {
    resource: String,
    size: f32,
    color: RgbaColor,
    /// Direction of the text space in visual page coordinates (see `content::Rewrite::setting`).
    direction: [f64; 4],
    /// Character codes, `bytes_per_char` per character.
    encoded: Vec<u8>,
    bytes_per_char: usize,
    /// Advance of each character in glyph space (1000 per em); `None` when unknown (standard fonts).
    widths: Option<Vec<f32>>,
    chars: Vec<char>,
}

/// Font used for one character of the new text: one of the PDF's own (by rank), an installed
/// one, or a standard one.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Own(usize),
    Installed,
    Standard,
}

/// An embedded font object, and the code and width of each character it draws.
type EmbeddedCodes = (u32, HashMap<char, (Vec<u8>, f32)>);

/// An installed font embedded once for the whole save, with every character any piece may take
/// from it, and shared by all those pieces.
struct SharedFace {
    face: InstalledFace,
    text: String,
    /// Once embedded (`None` inside: it could not be).
    embedded: Option<Option<EmbeddedCodes>>,
}

impl SharedFace {
    fn is(&self, face: &InstalledFace) -> bool {
        std::sync::Arc::ptr_eq(&self.face.data.0, &face.data.0) && self.face.index == face.index
    }

    fn embed(&mut self, update: &mut IncrementalUpdate<'_>) -> Option<&EmbeddedCodes> {
        if self.embedded.is_none() {
            let mut seen = HashSet::new();
            let unique: String = self.text.chars().filter(|c| seen.insert(*c)).collect();
            let text: String = unique
                .chars()
                .zip(installed_coverage(&self.face, &unique))
                .filter(|(_, ok)| *ok)
                .map(|(c, _)| c)
                .collect();
            self.embedded = Some(embed_truetype(&self.face, &text, update).map(|font| {
                let codes = text
                    .chars()
                    .zip(font.encoded.chunks(font.bytes_per_char))
                    .zip(&font.widths)
                    .map(|((c, code), w)| (c, (code.to_vec(), *w)))
                    .collect();
                (font.object, codes)
            }));
        }
        self.embedded.as_ref()?.as_ref()
    }
}

#[derive(Debug)]
pub enum EditError {
    Corrupt(String),
    /// Encrypted files would need their new objects encrypted too; not supported yet.
    Encrypted,
    PageOutOfRange(usize),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Corrupt(m) => write!(f, "PDF illisible : {m}"),
            Self::Encrypted => write!(
                f,
                "Ce PDF est protégé : la modification directe n’est pas encore possible."
            ),
            Self::PageOutOfRange(i) => write!(f, "Page {} introuvable", i + 1),
        }
    }
}

impl std::error::Error for EditError {}

/// The edited file, and how many edits had to fall back to covering the old text.
pub struct EditOutcome {
    pub pdf: Vec<u8>,
    pub covered_edits: usize,
    /// For each edit, in order, how far its new text reaches across the page: left and right edge
    /// in visual points, when every advance is known.
    pub extents: Vec<Option<(f32, f32)>>,
}

/// Apply `edits` to `pdf_bytes`. `doc` must be the extraction of the same bytes (its glyph indices
/// are what [`TextEdit::glyph_indices`] refer to).
pub fn apply_text_edits(pdf_bytes: &[u8], doc: &DocIr, edits: &[TextEdit]) -> Result<EditOutcome, EditError> {
    let trailer = read_trailer(pdf_bytes).ok_or_else(|| EditError::Corrupt("trailer introuvable".into()))?;
    if trailer.encrypted {
        return Err(EditError::Encrypted);
    }
    if edits.is_empty() {
        return Ok(EditOutcome {
            pdf: pdf_bytes.to_vec(),
            covered_edits: 0,
            extents: Vec::new(),
        });
    }
    let pdf = Pdf::new(pdf_bytes.to_vec()).map_err(|e| EditError::Corrupt(format!("{e:?}")))?;
    let pages = pdf.pages();
    let mut update = IncrementalUpdate::new(pdf_bytes, trailer);

    // One font object per standard face in use, shared by all pages, made when first needed.
    let mut standard: Vec<(&'static str, u32)> = Vec::new();
    let mut shared: Vec<SharedFace> = Vec::new();
    for piece in edits.iter().flat_map(|e| &e.pieces) {
        let Some(face) = &piece.fallback else {
            continue;
        };
        match shared.iter_mut().find(|f| f.is(face)) {
            Some(f) => f.text.push_str(&piece.text),
            None => shared.push(SharedFace {
                face: face.clone(),
                text: piece.text.clone(),
                embedded: None,
            }),
        }
    }

    let mut covered_edits = 0;
    let mut extents = vec![None; edits.len()];
    let page_indices: BTreeSet<usize> = edits.iter().map(|e| e.page_index).collect();
    for page_index in page_indices {
        let page = pages.get(page_index).ok_or(EditError::PageOutOfRange(page_index))?;
        let page_ir = doc.pages.get(page_index).ok_or(EditError::PageOutOfRange(page_index))?;
        let page_dict = page.raw();
        let page_id = page_dict
            .obj_id()
            .ok_or_else(|| EditError::Corrupt("page sans numéro d’objet".into()))?;
        let page_edits: Vec<(usize, &TextEdit)> = edits.iter().enumerate().filter(|(_, e)| e.page_index == page_index).collect();
        let hide: BTreeSet<usize> = page_edits
            .iter()
            .flat_map(|(_, e)| e.glyph_indices.iter().copied())
            .collect();
        let page_tf = page.initial_transform(true).to_kurbo();
        let original = page.page_stream().unwrap_or(&[]);
        let rewrite = Walker::new(&page_ir.glyphs, &hide, page_tf).rewrite_page(original, page.resources());

        let forms = copy_forms(page.resources(), &rewrite.forms, &mut update);
        // A glyph is gone once its rewritten content is what the page draws.
        let removed_glyph = |i: &usize| {
            rewrite.hidden.contains(i)
                && rewrite.fonts.get(i).is_some_and(|(path, _)| path.is_empty() || forms.wired.contains(path))
        };

        let to_user = page_tf.inverse();
        let fonts_raw = non_empty(page.resources().fonts.data()).unwrap_or(b"<< >>".to_vec());
        let taken: Vec<Vec<u8>> = lexer::dict_entries(&fonts_raw)
            .unwrap_or_default()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        // Fonts the new text uses on this page, by their new resource name.
        let mut added: Vec<(String, u32)> = Vec::new();
        let mut stream = b"q\n".to_vec();
        stream.extend_from_slice(&rewrite.content);
        stream.extend_from_slice(b"\nQ\n");
        for &(edit_index, e) in &page_edits {
            let removed = e.glyph_indices.iter().all(removed_glyph);
            if !removed {
                covered_edits += 1;
                let b = e.cover_bbox;
                let r = to_user.transform_rect_bbox(Rect::new(b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64));
                stream.extend_from_slice(
                    format!(
                        "q {} rg {} {} {} {} re f Q\n",
                        rgb(e.cover_color),
                        fmt_num(r.x0),
                        fmt_num(r.y0),
                        fmt_num(r.width()),
                        fmt_num(r.height())
                    )
                    .as_bytes(),
                );
            }
            let mut runs: Vec<Run> = Vec::new();
            for piece in e.pieces.iter().filter(|p| !p.text.is_empty()) {
                let font_of = |glyph: usize| {
                    let (path, name) = rewrite.fonts.get(&glyph)?;
                    form_resources(page.resources(), path)?.get_font(&Name::new_unescaped(name))
                };
                // The PDF's own fonts for this style, set as the content stream sets them.
                let owns: Vec<OwnFont<'_, '_>> = piece
                    .glyph_index
                    .map(|g| (g, piece.known_chars.as_str()))
                    .into_iter()
                    .chain(piece.other_fonts.iter().map(|(g, known)| (*g, known.as_str())))
                    .filter_map(|(glyph, known)| {
                        // The codes the edited line drew in this very font, by character.
                        let font = rewrite.fonts.get(&glyph)?;
                        let line_codes = e
                            .glyph_indices
                            .iter()
                            .filter(|i| rewrite.fonts.get(i) == Some(font))
                            .filter_map(|i| {
                                let mut chars = page_ir.glyphs.get(*i)?.unicode.chars();
                                let c = chars.next().filter(|_| chars.next().is_none())?;
                                let code = rewrite.codes.get(i)?;
                                Some((c, code.iter().fold(0u32, |acc, &b| (acc << 8) | b as u32)))
                            })
                            .collect();
                        Some(OwnFont {
                            dict: font_of(glyph)?,
                            usage: FontUse { known, line_codes },
                            setting: rewrite.setting.get(&glyph).copied(),
                        })
                    })
                    .collect();
                let fonts = Fonts {
                    added: &mut added,
                    taken: &taken,
                    shared: &mut shared,
                    standard: &mut standard,
                };
                runs.extend(typeset(piece, &owns, &mut update, fonts));
            }
            if runs.is_empty() {
                continue;
            }
            let th = if e.horiz_scale > 0.05 { e.horiz_scale } else { 1.0 };
            let width = line_width(e, &runs, th);
            let x = match (e.align, width) {
                (Alignment::Right, Some(w)) => e.x1_pt - w,
                (Alignment::Center, Some(w)) => 0.5 * (e.x_pt + e.x1_pt) - 0.5 * w,
                _ => e.x_pt,
            };
            extents[edit_index] = width.map(|w| (x, x + w));
            stream.extend_from_slice(
                format!(
                    "BT {} Tc {} Tz",
                    fmt_num((e.char_spacing_pt / th) as f64),
                    fmt_num((th * 100.0) as f64)
                )
                .as_bytes(),
            );
            // Each run starts where the previous one ends; a new text matrix is only needed where
            // the direction of the text space changes (fonts drawn upside down by design).
            let mut pen = Some(x);
            let mut direction = None;
            let mut color = None;
            for run in &runs {
                if direction != Some(run.direction)
                    && let Some(px) = pen
                {
                    let d = run.direction;
                    let tm = to_user * Affine::new([d[0], d[1], d[2], d[3], px as f64, e.baseline_pt as f64]);
                    let c = tm.as_coeffs();
                    stream.extend_from_slice(
                        format!(
                            " {} {} {} {} {} {} Tm",
                            fmt_num(c[0]),
                            fmt_num(c[1]),
                            fmt_num(c[2]),
                            fmt_num(c[3]),
                            fmt_num(c[4]),
                            fmt_num(c[5])
                        )
                        .as_bytes(),
                    );
                    direction = Some(run.direction);
                }
                pen = pen.zip(run_advance(e, run, th)).map(|(p, w)| p + w);
                if color != Some(run.color) {
                    stream.extend_from_slice(format!(" {} rg", rgb(run.color)).as_bytes());
                    color = Some(run.color);
                }
                stream.extend_from_slice(format!(" /{} {} Tf [", run.resource, fmt_num(run.size as f64)).as_bytes());
                // Word spacing as a position adjustment after each space: `Tw` does not apply to
                // the two-byte codes of CID fonts.
                let adjust = -e.word_spacing_pt * 1000.0 / (run.size * th);
                let mut start = 0;
                for (k, ch) in run.chars.iter().enumerate() {
                    if ch.is_whitespace() && adjust.abs() > 0.01 {
                        let end = (k + 1) * run.bytes_per_char;
                        write_hex_string(&run.encoded[start..end], &mut stream);
                        stream.extend_from_slice(format!(" {} ", fmt_num(adjust.round() as f64)).as_bytes());
                        start = end;
                    }
                }
                if start < run.encoded.len() {
                    write_hex_string(&run.encoded[start..], &mut stream);
                }
                stream.extend_from_slice(b"] TJ");
            }
            stream.extend_from_slice(b" ET\n");
        }
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&stream, 6);
        let mut content_obj = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", compressed.len()).into_bytes();
        content_obj.extend_from_slice(&compressed);
        content_obj.extend_from_slice(b"\nendstream");
        let content_num = update.add_object(content_obj);

        let resources = page_resources_raw(page_dict, page.resources());
        let new_fonts: Vec<(Vec<u8>, Vec<u8>)> = added
            .iter()
            .map(|(name, num)| (name.clone().into_bytes(), format!("{num} 0 R").into_bytes()))
            .collect();
        let new_fonts_ref: Vec<(&[u8], Vec<u8>)> = new_fonts.iter().map(|(k, v)| (k.as_slice(), v.clone())).collect();
        let font_dict =
            patch_dict(&fonts_raw, &new_fonts_ref, &[]).ok_or_else(|| EditError::Corrupt("polices".into()))?;
        let mut resource_patches: Vec<(&[u8], Vec<u8>)> = vec![(b"Font", font_dict)];
        if !forms.entries.is_empty() {
            let x_objects = non_empty(page.resources().x_objects.data()).unwrap_or(b"<< >>".to_vec());
            let forms_ref: Vec<(&[u8], Vec<u8>)> = forms.entries.iter().map(|(k, v)| (k.as_slice(), v.clone())).collect();
            let x_object_dict =
                patch_dict(&x_objects, &forms_ref, &[]).ok_or_else(|| EditError::Corrupt("XObject".into()))?;
            resource_patches.push((b"XObject", x_object_dict));
        }
        let resources_dict =
            patch_dict(&resources, &resource_patches, &[]).ok_or_else(|| EditError::Corrupt("ressources".into()))?;
        let new_page = patch_dict(
            page_dict.data(),
            &[
                (b"Contents", format!("{content_num} 0 R").into_bytes()),
                (b"Resources", resources_dict),
            ],
            &[],
        )
        .ok_or_else(|| EditError::Corrupt("page".into()))?;
        update.set_object(page_id.obj_number as u32, page_id.gen_number as u16, new_page);
    }

    Ok(EditOutcome {
        pdf: update.finish(),
        covered_edits,
        extents,
    })
}

/// Fonts the new text of a page can use: derived from the page's, installed ones embedded for
/// the whole save, standard ones; and the resource names given to them on this page.
struct Fonts<'f> {
    added: &'f mut Vec<(String, u32)>,
    /// Resource names the page's fonts already have (earlier saves included).
    taken: &'f [Vec<u8>],
    shared: &'f mut Vec<SharedFace>,
    standard: &'f mut Vec<(&'static str, u32)>,
}

impl Fonts<'_> {
    /// Resource name of the font object `object` on this page. It is named after the object, a
    /// new one that no earlier save could have named, and kept clear of the page's own names.
    fn resource(&mut self, prefix: &str, object: u32) -> String {
        if let Some((name, _)) = self.added.iter().find(|(_, o)| *o == object) {
            return name.clone();
        }
        let mut name = format!("{prefix}{object}");
        while self.taken.iter().any(|k| k == name.as_bytes()) {
            name.push('x');
        }
        self.added.push((name.clone(), object));
        name
    }
}

/// Split a piece of the new text into runs of one font each: the PDF's own fonts (`owns`, in
/// turn) for the characters they have, the installed fallback for the others (or for all of them
/// when it is the same family), and a standard font for whatever is left.
fn typeset(e: &TextPiece, owns: &[OwnFont<'_, '_>], update: &mut IncrementalUpdate<'_>, fonts: Fonts<'_>) -> Vec<Run> {
    const UPRIGHT: [f64; 4] = [1.0, 0.0, 0.0, -1.0];
    let mut fonts = fonts;
    let chars: Vec<char> = e.text.chars().collect();
    let coverages: Vec<Vec<bool>> = owns.iter().map(|o| coverage(&o.dict, &e.text, &o.usage)).collect();
    let own: Vec<Option<usize>> = (0..chars.len())
        .map(|k| coverages.iter().position(|c| c[k]))
        .collect();
    let installed = e
        .fallback
        .as_ref()
        .map_or_else(|| vec![false; chars.len()], |f| installed_coverage(f, &e.text));
    // Only when it can be embedded: otherwise the PDF's own fonts keep the characters they have.
    let whole_piece_installed = e.fallback_same_family
        && own.contains(&None)
        && !installed.contains(&false)
        && e.fallback.as_ref().is_some_and(|face| {
            fonts.shared.iter_mut().find(|f| f.is(face)).and_then(|f| f.embed(update)).is_some()
        });
    let sources: Vec<Source> = (0..chars.len())
        .map(|k| match own[k] {
            Some(i) if !whole_piece_installed => Source::Own(i),
            _ if installed[k] => Source::Installed,
            _ => Source::Standard,
        })
        .collect();

    // Each of the PDF's fonts, derived once for all the characters it draws; they take its codes
    // in order.
    let mut derived: Vec<Option<(DerivedFont, String, usize)>> = owns
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let text: String =
                chars.iter().zip(&sources).filter(|(_, s)| **s == Source::Own(i)).map(|(c, _)| *c).collect();
            if text.is_empty() {
                return None;
            }
            derive_font(&o.dict, &text, &o.usage, update).map(|mut d| {
                // A font that declares no width for a character: use the one the reader draws it with.
                for (w, c) in d.widths.iter_mut().zip(text.chars()) {
                    if *w <= 0.0
                        && let Some((_, known)) = e.char_widths.iter().find(|(k, _)| *k == c)
                    {
                        *w = *known;
                    }
                }
                let name = fonts.resource("P2DR", d.object);
                (d, name, 0usize)
            })
        })
        .collect();

    let mut runs: Vec<Run> = Vec::new();
    for (&c, src) in chars.iter().zip(&sources) {
        let found = match *src {
            Source::Own(i) => derived[i].as_mut().map(|(font, name, next)| {
                let b = font.bytes_per_char;
                let code = font.encoded[*next * b..(*next + 1) * b].to_vec();
                let width = font.widths.get(*next).copied();
                *next += 1;
                let setting = owns[i].setting;
                let size = setting.map_or(e.nominal_font_size_pt, |(size, _)| size as f32);
                (name.clone(), size, code, width, setting.map_or(UPRIGHT, |(_, d)| d))
            }),
            Source::Installed => {
                let face = e.fallback.as_ref();
                let shared = face.and_then(|face| fonts.shared.iter_mut().find(|f| f.is(face)));
                let embedded = shared.and_then(|f| f.embed(update)).cloned();
                embedded.and_then(|(object, codes)| {
                    let (code, width) = codes.get(&c)?.clone();
                    Some((fonts.resource("P2DR", object), e.font_size_pt, code, Some(width), UPRIGHT))
                })
            }
            Source::Standard => None,
        };
        let (resource, size, code, width, direction) = found.unwrap_or_else(|| {
            let base = e.family.base_font(e.bold, e.italic);
            let num = match fonts.standard.iter().find(|(b, _)| *b == base) {
                Some((_, num)) => *num,
                None => {
                    let num = update.add_object(
                        format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Encoding /WinAnsiEncoding >>")
                            .into_bytes(),
                    );
                    fonts.standard.push((base, num));
                    num
                }
            };
            (fonts.resource("P2DF", num), e.font_size_pt, to_win_ansi(&c.to_string()), None, UPRIGHT)
        });
        match runs.last_mut() {
            Some(run) if run.resource == resource => {
                run.encoded.extend_from_slice(&code);
                run.chars.push(c);
                run.widths = run.widths.take().zip(width).map(|(mut ws, w)| {
                    ws.push(w);
                    ws
                });
            }
            _ => runs.push(Run {
                resource,
                size,
                color: e.color,
                direction,
                bytes_per_char: code.len(),
                encoded: code,
                widths: width.map(|w| vec![w]),
                chars: vec![c],
            }),
        }
    }
    runs
}

/// How far a run moves the pen, in visual points, when its advances are known.
fn run_advance(e: &TextEdit, run: &Run, th: f32) -> Option<f32> {
    let widths = run.widths.as_ref()?;
    Some(
        run.chars
            .iter()
            .zip(widths)
            .map(|(c, adv)| {
                let space = if c.is_whitespace() { e.word_spacing_pt } else { 0.0 };
                adv / 1000.0 * run.size * th + e.char_spacing_pt + space
            })
            .sum(),
    )
}

/// Width of the typeset line in visual points, from the first glyph's origin to the end of the
/// last one's advance (without the character spacing after it), when every advance is known.
fn line_width(e: &TextEdit, runs: &[Run], th: f32) -> Option<f32> {
    runs.iter()
        .map(|run| run_advance(e, run, th))
        .sum::<Option<f32>>()
        .map(|w| w - e.char_spacing_pt)
}

/// Resources in effect inside the form reached from the page by the `/XObject` keys of `path`.
fn form_resources<'a>(page: &Resources<'a>, path: &[Vec<u8>]) -> Option<Resources<'a>> {
    let mut res = page.clone();
    for name in path {
        let form = res.get_x_object(&Name::new_unescaped(name))?;
        if let Some(own) = form.dict().get::<Dict<'_>>(RESOURCES) {
            res = Resources::new(own);
        }
    }
    Some(res)
}

/// The `/XObject` names leading from the page to a form.
type FormPath = Vec<Vec<u8>>;

/// Copies of a page's forms, made by [`copy_forms`].
struct FormCopies {
    /// The page-level `/XObject` entries to replace.
    entries: Vec<(Vec<u8>, Vec<u8>)>,
    /// The rewritten forms whose copy the page now reaches.
    wired: BTreeSet<FormPath>,
}

/// New copies of the forms whose text was rewritten, and of every form leading to them (pointing
/// at the copies below them), so this page alone sees the change.
fn copy_forms(page: &Resources<'_>, rewritten: &[(FormPath, Vec<u8>)], update: &mut IncrementalUpdate<'_>) -> FormCopies {
    // Every form on the way to a rewritten one, with its new content when it has one.
    let mut copies: BTreeMap<Vec<Vec<u8>>, Option<&[u8]>> = BTreeMap::new();
    for (path, content) in rewritten {
        for len in 1..path.len() {
            copies.entry(path[..len].to_vec()).or_insert(None);
        }
        copies.insert(path.clone(), Some(content));
    }
    // Deepest first, so a form's copy can point at its children's copies.
    let mut order: Vec<Vec<Vec<u8>>> = copies.keys().cloned().collect();
    order.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let mut made: BTreeMap<Vec<Vec<u8>>, u32> = BTreeMap::new();
    for path in order {
        let Some((name, parent)) = path.split_last() else {
            continue;
        };
        let Some(form) = form_resources(page, parent).and_then(|r| r.get_x_object(&Name::new_unescaped(name))) else {
            continue;
        };
        let dict = form.dict();
        let content = match copies[&path] {
            Some(c) => c.to_vec(),
            None => match form.decoded() {
                Ok(d) => d.to_vec(),
                Err(_) => continue,
            },
        };
        let children: Vec<(Vec<u8>, Vec<u8>)> = made
            .iter()
            .filter(|(p, _)| p.len() == path.len() + 1 && p.starts_with(&path))
            .map(|(p, num)| (p[path.len()].clone(), format!("{num} 0 R").into_bytes()))
            .collect();
        let mut replace: Vec<(&[u8], Vec<u8>)> = Vec::new();
        if !children.is_empty() {
            // A form without resources of its own uses those where it is drawn: its copy gets
            // them explicitly, pointing at the copies below it.
            let (res, x_objects) = match dict.get::<Dict<'_>>(RESOURCES) {
                Some(res) => (res.data().to_vec(), res.get::<Dict<'_>>(b"XObject").and_then(|d| non_empty(d.data()))),
                None => {
                    let Some(res) = form_resources(page, parent) else {
                        continue;
                    };
                    (resources_raw(&res), non_empty(res.x_objects.data()))
                }
            };
            let refs: Vec<(&[u8], Vec<u8>)> = children.iter().map(|(k, v)| (k.as_slice(), v.clone())).collect();
            let Some(x_objects) = patch_dict(&x_objects.unwrap_or(b"<< >>".to_vec()), &refs, &[]) else {
                continue;
            };
            let Some(res) = patch_dict(&res, &[(b"XObject", x_objects)], &[]) else {
                continue;
            };
            replace.push((b"Resources", res));
        }
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&content, 6);
        replace.push((b"Length", compressed.len().to_string().into_bytes()));
        replace.push((b"Filter", b"/FlateDecode".to_vec()));
        let Some(mut obj) = patch_dict(dict.data(), &replace, &[b"DecodeParms", b"F", b"FFilter", b"FDecodeParms", b"DL"])
        else {
            continue;
        };
        obj.extend_from_slice(b"\nstream\n");
        obj.extend_from_slice(&compressed);
        obj.extend_from_slice(b"\nendstream");
        made.insert(path, update.add_object(obj));
    }
    let wired = rewritten
        .iter()
        .map(|(path, _)| path)
        .filter(|path| (1..=path.len()).all(|len| made.contains_key(&path[..len])))
        .cloned()
        .collect();
    let entries = made
        .into_iter()
        .filter(|(p, _)| p.len() == 1)
        .map(|(p, num)| (p[0].clone(), format!("{num} 0 R").into_bytes()))
        .collect();
    FormCopies { entries, wired }
}

fn rgb(c: RgbaColor) -> String {
    let (r, g, b) = c.composited_over_white();
    format!(
        "{} {} {}",
        fmt_num((r as f64 / 255.0 * 1000.0).round() / 1000.0),
        fmt_num((g as f64 / 255.0 * 1000.0).round() / 1000.0),
        fmt_num((b as f64 / 255.0 * 1000.0).round() / 1000.0)
    )
}

fn non_empty(data: &[u8]) -> Option<Vec<u8>> {
    let trimmed = data.iter().position(|b| !b.is_ascii_whitespace())?;
    data[trimmed..].starts_with(b"<<").then(|| data.to_vec())
}

/// The page's resource dictionary as raw bytes, rebuilt from the inherited parts when the page
/// dictionary has none of its own.
fn page_resources_raw(page_dict: &Dict<'_>, res: &Resources<'_>) -> Vec<u8> {
    if let Some(own) = page_dict.get::<Dict<'_>>(RESOURCES).and_then(|d| non_empty(d.data())) {
        return own;
    }
    resources_raw(res)
}

/// A resource dictionary as raw bytes, rebuilt from its parts.
fn resources_raw(res: &Resources<'_>) -> Vec<u8> {
    let mut out = b"<<".to_vec();
    for (key, dict) in [
        ("ExtGState", &res.ext_g_states),
        ("Font", &res.fonts),
        ("Properties", &res.properties),
        ("ColorSpace", &res.color_spaces),
        ("XObject", &res.x_objects),
        ("Pattern", &res.patterns),
        ("Shading", &res.shadings),
    ] {
        if let Some(raw) = non_empty(dict.data()) {
            out.extend_from_slice(format!("/{key} ").as_bytes());
            out.extend_from_slice(&raw);
            out.push(b' ');
        }
    }
    out.extend_from_slice(b">>");
    out
}

/// Encode text for the standard fonts' `WinAnsiEncoding` (characters outside it become `?`).
pub fn to_win_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match c {
            '\u{0020}'..='\u{007E}' => c as u8,
            '\u{00A0}' | '\u{202F}' => 0xA0,
            '\u{00A1}'..='\u{00FF}' => c as u32 as u8,
            '€' => 0x80,
            '‚' => 0x82,
            'ƒ' => 0x83,
            '„' => 0x84,
            '…' => 0x85,
            '†' => 0x86,
            '‡' => 0x87,
            'ˆ' => 0x88,
            '‰' => 0x89,
            'Š' => 0x8A,
            '‹' => 0x8B,
            'Œ' => 0x8C,
            'Ž' => 0x8E,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '˜' => 0x98,
            '™' => 0x99,
            'š' => 0x9A,
            '›' => 0x9B,
            'œ' => 0x9C,
            'ž' => 0x9E,
            'Ÿ' => 0x9F,
            _ => b'?',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pdf_writer::{Content, Finish, Name as PdfName, Pdf as PdfWriter, Rect as PdfRect, Ref, Str};

    use super::*;
    use crate::{ExtractOptions, HayroBackend, PdfBackend};

    /// One page, two lines: a plain `Tj` and a `TJ` array with spacing adjustments.
    fn two_line_pdf() -> Vec<u8> {
        let mut pdf = PdfWriter::new();
        let (catalog, tree, font, page, content_id) = (Ref::new(1), Ref::new(2), Ref::new(3), Ref::new(4), Ref::new(5));
        pdf.catalog(catalog).pages(tree);
        pdf.type1_font(font)
            .base_font(PdfName(b"Helvetica"))
            .encoding_predefined(PdfName(b"WinAnsiEncoding"));
        let mut c = Content::new();
        c.begin_text();
        c.set_font(PdfName(b"F1"), 12.0);
        c.next_line(72.0, 700.0);
        c.show(Str(b"Madame Marie Martin"));
        c.next_line(0.0, -20.0);
        c.show_positioned()
            .items()
            .show(Str(b"Total : "))
            .adjust(-200.0)
            .show(Str(b"42,00 EUR"));
        c.end_text();
        pdf.stream(content_id, &c.finish());
        let mut p = pdf.page(page);
        p.parent(tree)
            .media_box(PdfRect::new(0.0, 0.0, 595.0, 842.0))
            .contents(content_id);
        p.resources().fonts().pair(PdfName(b"F1"), font);
        p.finish();
        pdf.pages(tree).kids([page]).count(1);
        pdf.finish()
    }

    fn extract(bytes: &[u8]) -> DocIr {
        HayroBackend::new()
            .extract_document(Arc::from(bytes), None, &ExtractOptions::full())
            .unwrap()
            .0
    }

    fn visible_text_at(doc: &DocIr, baseline: f32) -> String {
        let mut glyphs: Vec<&p2d_ir::GlyphIr> = doc.pages[0]
            .glyphs
            .iter()
            .filter(|g| (g.baseline_y - baseline).abs() < 0.5 && g.render_mode != p2d_ir::RenderMode::Invisible)
            .collect();
        glyphs.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
        glyphs.iter().map(|g| g.unicode.as_str()).collect()
    }

    fn edit_line(doc: &DocIr, baseline: f32, text: &str) -> TextEdit {
        let page = &doc.pages[0];
        let indices: Vec<usize> = (0..page.glyphs.len())
            .filter(|&i| (page.glyphs[i].baseline_y - baseline).abs() < 0.5)
            .collect();
        let x = page.glyphs[indices[0]].bbox.x0;
        TextEdit {
            page_index: 0,
            glyph_indices: indices.clone(),
            pieces: vec![TextPiece {
                text: text.to_string(),
                glyph_index: indices.first().copied(),
                font_size_pt: 12.0,
                nominal_font_size_pt: 12.0,
                color: RgbaColor::BLACK,
                family: StandardFamily::Helvetica,
                bold: false,
                italic: false,
                fallback: None,
                fallback_same_family: false,
                known_chars: String::new(),
                other_fonts: Vec::new(),
                char_widths: Vec::new(),
            }],
            x_pt: x,
            baseline_pt: baseline,
            x1_pt: x + 100.0,
            align: Alignment::Left,
            char_spacing_pt: 0.0,
            word_spacing_pt: 0.0,
            horiz_scale: 1.0,
            cover_bbox: RectPt::new(x, baseline - 10.0, x + 200.0, baseline + 3.0),
            cover_color: RgbaColor::WHITE,
        }
    }

    #[test]
    fn test_edit_rewrites_only_the_chosen_line() {
        let original = two_line_pdf();
        let doc = extract(&original);
        let first_baseline = doc.pages[0].glyphs[0].baseline_y;
        let second_baseline = first_baseline + 20.0;
        assert_eq!(visible_text_at(&doc, second_baseline), "Total : 42,00 EUR");

        let edit = edit_line(&doc, second_baseline, "Total : 99,90 EUR");
        let outcome = apply_text_edits(&original, &doc, &[edit]).unwrap();
        assert_eq!(outcome.covered_edits, 0, "the TJ line must be rewritten, not covered");
        assert!(
            outcome.pdf.starts_with(&original),
            "incremental update keeps the original bytes"
        );

        let edited = extract(&outcome.pdf);
        assert_eq!(visible_text_at(&edited, first_baseline), "Madame Marie Martin");
        assert_eq!(visible_text_at(&edited, second_baseline), "Total : 99,90 EUR");
        let hidden = edited.pages[0]
            .glyphs
            .iter()
            .filter(|g| g.render_mode == p2d_ir::RenderMode::Invisible)
            .count();
        assert_eq!(
            hidden,
            "Total : 42,00 EUR".len(),
            "old glyphs stay in place but invisible"
        );
    }

    #[test]
    fn test_second_edit_stacks_on_the_first() {
        let original = two_line_pdf();
        let doc = extract(&original);
        let baseline = doc.pages[0].glyphs[0].baseline_y;
        let first = apply_text_edits(&original, &doc, &[edit_line(&doc, baseline, "Monsieur Jean Martin")]).unwrap();
        let doc2 = extract(&first.pdf);
        // The replacement line is now made of new glyphs: edit it again.
        let page = &doc2.pages[0];
        let indices: Vec<usize> = (0..page.glyphs.len())
            .filter(|&i| {
                (page.glyphs[i].baseline_y - baseline).abs() < 0.5
                    && page.glyphs[i].render_mode != p2d_ir::RenderMode::Invisible
            })
            .collect();
        let mut edit = edit_line(&doc2, baseline, "M. Jean Martin");
        edit.glyph_indices = indices;
        let second = apply_text_edits(&first.pdf, &doc2, &[edit]).unwrap();
        assert!(second.pdf.starts_with(&first.pdf));
        assert_eq!(visible_text_at(&extract(&second.pdf), baseline), "M. Jean Martin");
    }

    #[test]
    fn test_win_ansi_encoding() {
        assert_eq!(to_win_ansi("é€œ"), vec![0xE9, 0x80, 0x9C]);
        assert_eq!(to_win_ansi("\u{202F}"), vec![0xA0]);
        assert_eq!(to_win_ansi("→"), b"?".to_vec());
    }

    /// A PDF made of these object bodies, object `i + 1` being `objects[i]`: 1 is the catalog,
    /// 2 the page tree, 3 the single page.
    fn raw_pdf(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
        );
        out
    }

    fn obj(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    fn stream_obj(dict: &str, data: &str) -> Vec<u8> {
        format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len()).into_bytes()
    }

    fn one_page(resources: &str, content: &str, more: Vec<Vec<u8>>) -> Vec<u8> {
        let mut objects = vec![
            obj("<< /Type /Catalog /Pages 2 0 R >>"),
            obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            obj(&format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources {resources} >>")),
            stream_obj("", content),
        ];
        objects.extend(more);
        raw_pdf(&objects)
    }

    /// Glyphs of `doc`'s page drawn visibly on `baseline`, left to right.
    fn visible_glyphs(doc: &DocIr, baseline: f32) -> Vec<p2d_ir::GlyphIr> {
        let mut glyphs: Vec<p2d_ir::GlyphIr> = doc.pages[0]
            .glyphs
            .iter()
            .filter(|g| (g.baseline_y - baseline).abs() < 0.5 && g.render_mode != p2d_ir::RenderMode::Invisible)
            .cloned()
            .collect();
        glyphs.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
        glyphs
    }

    #[test]
    fn test_right_aligned_edit_keeps_its_edge_and_letter_spacing() {
        // "12,50" in Helvetica 12 with 0.5 pt letter spacing, ending at x = 432.024.
        let original = one_page(
            "<< /Font << /F1 5 0 R >> >>",
            "BT /F1 12 Tf 0.5 Tc 1 0 0 1 400 700 Tm (12,50) Tj ET",
            vec![obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")],
        );
        let doc = extract(&original);
        let baseline = doc.pages[0].glyphs[0].baseline_y;
        let x1 = {
            let g = visible_glyphs(&doc, baseline);
            let last = g.last().unwrap();
            last.bbox.x0 + last.advance_pt
        };
        assert!((x1 - 432.024).abs() < 0.01, "{x1}");
        let mut edit = edit_line(&doc, baseline, "112,50");
        edit.align = Alignment::Right;
        edit.x1_pt = x1;
        edit.char_spacing_pt = 0.5;
        // Helvetica does not declare its widths in the PDF: the caller tells them.
        edit.pieces[0].char_widths = vec![('1', 556.0), ('2', 556.0), ('5', 556.0), ('0', 556.0), (',', 278.0)];
        let outcome = apply_text_edits(&original, &doc, &[edit]).unwrap();
        assert_eq!(outcome.covered_edits, 0);

        let edited = extract(&outcome.pdf);
        let glyphs = visible_glyphs(&edited, baseline);
        assert_eq!(glyphs.iter().map(|g| g.unicode.as_str()).collect::<String>(), "112,50");
        let last = glyphs.last().unwrap();
        assert!((last.bbox.x0 + last.advance_pt - x1).abs() < 0.05, "right edge moved");
        for w in glyphs.windows(2) {
            let gap = w[1].bbox.x0 - (w[0].bbox.x0 + w[0].advance_pt);
            assert!((gap - 0.5).abs() < 0.02, "letter spacing {gap}");
        }
        // The editor fits the line's clickable zone to where the new text is drawn.
        let (left, right) = outcome.extents[0].expect("every advance is known");
        assert!((left - glyphs[0].bbox.x0).abs() < 0.05 && (right - x1).abs() < 0.05, "{left}..{right}");
        assert!(!String::from_utf8_lossy(&outcome.pdf[original.len()..]).contains("/BaseFont /Helvetica-Bold"));
    }

    #[test]
    fn test_text_in_nested_forms_is_rewritten_not_covered() {
        // page -> /Outer form -> /Inner form -> the text.
        let original = one_page(
            "<< /XObject << /Outer 5 0 R >> >>",
            "q /Outer Do Q",
            vec![
                stream_obj(
                    "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Resources << /XObject << /Inner 6 0 R >> >>",
                    "/Inner Do",
                ),
                stream_obj(
                    "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Resources << /Font << /F1 7 0 R >> >>",
                    "BT /F1 12 Tf 72 700 Td (Hello world) Tj ET",
                ),
                obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"),
            ],
        );
        let doc = extract(&original);
        let baseline = doc.pages[0].glyphs[0].baseline_y;
        let outcome = apply_text_edits(&original, &doc, &[edit_line(&doc, baseline, "Bonjour")]).unwrap();
        assert_eq!(outcome.covered_edits, 0, "the form text must be rewritten, not covered");
        let edited = extract(&outcome.pdf);
        assert_eq!(visible_text_at(&edited, baseline), "Bonjour");
        // The original form objects are untouched: the page draws rewritten copies.
        assert!(outcome.pdf.starts_with(&original));
    }

    #[test]
    fn test_each_save_names_its_fonts_anew() {
        // A Helvetica line and a Courier line, edited in two saves: the second save's font must
        // not take the name the first save gave to the Helvetica copy.
        let original = one_page(
            "<< /Font << /F1 5 0 R /F2 6 0 R >> >>",
            "BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F2 12 Tf 72 680 Td (World) Tj ET",
            vec![
                obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"),
                obj("<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>"),
            ],
        );
        let doc = extract(&original);
        let top = doc.pages[0].glyphs[0].baseline_y;
        let first = apply_text_edits(&original, &doc, &[edit_line(&doc, top, "Hallo")]).unwrap();
        let doc1 = extract(&first.pdf);
        let second = apply_text_edits(&first.pdf, &doc1, &[edit_line(&doc1, top + 20.0, "Welt")]).unwrap();
        let edited = extract(&second.pdf);
        let font_of = |baseline: f32| {
            let glyphs = visible_glyphs(&edited, baseline);
            edited.pages[0].fonts[glyphs[0].font_idx].postscript_name.clone()
        };
        assert_eq!(visible_text_at(&edited, top), "Hallo");
        assert_eq!(visible_text_at(&edited, top + 20.0), "Welt");
        assert_eq!(font_of(top), "Helvetica");
        assert_eq!(font_of(top + 20.0), "Courier");
    }

    #[test]
    fn test_form_without_resources_on_the_way_is_rewritten() {
        // page -> /Outer (no resources: it uses the page's) -> /Inner -> the text.
        let original = one_page(
            "<< /XObject << /Outer 5 0 R /Inner 6 0 R >> >>",
            "q /Outer Do Q",
            vec![
                stream_obj("/Type /XObject /Subtype /Form /BBox [0 0 595 842]", "/Inner Do"),
                stream_obj(
                    "/Type /XObject /Subtype /Form /BBox [0 0 595 842] /Resources << /Font << /F1 7 0 R >> >>",
                    "BT /F1 12 Tf 72 700 Td (Hello world) Tj ET",
                ),
                obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"),
            ],
        );
        let doc = extract(&original);
        let baseline = doc.pages[0].glyphs[0].baseline_y;
        let outcome = apply_text_edits(&original, &doc, &[edit_line(&doc, baseline, "Bonjour")]).unwrap();
        assert_eq!(outcome.covered_edits, 0);
        assert_eq!(visible_text_at(&extract(&outcome.pdf), baseline), "Bonjour");
    }

    #[test]
    fn test_stretch_from_the_text_matrix_is_applied_once() {
        let original = one_page(
            "<< /Font << /F1 5 0 R >> >>",
            "BT /F1 12 Tf 1.5 0 0 1 72 700 Tm (Hello) Tj ET",
            vec![obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")],
        );
        let doc = extract(&original);
        let baseline = doc.pages[0].glyphs[0].baseline_y;
        let before: Vec<f32> = visible_glyphs(&doc, baseline).iter().map(|g| g.advance_pt).collect();
        let mut edit = edit_line(&doc, baseline, "Hello");
        edit.horiz_scale = 1.5;
        let outcome = apply_text_edits(&original, &doc, &[edit]).unwrap();
        let after: Vec<f32> = visible_glyphs(&extract(&outcome.pdf), baseline).iter().map(|g| g.advance_pt).collect();
        assert_eq!(before.len(), after.len());
        for (b, a) in before.iter().zip(&after) {
            assert!((a - b).abs() < 0.05, "advance {a} instead of {b}");
        }
    }

    /// A Type 3 font drawing `a` and `b` as boxes, with these widths and font matrix scale.
    fn type3_font(name: &str, widths: [u32; 2], scale: f32, procs: (u32, u32), to_unicode: u32) -> Vec<u8> {
        obj(&format!(
            "<< /Type /Font /Subtype /Type3 /Name /{name} /FontBBox [0 0 1000 1000] /FontMatrix [{scale} 0 0 {scale} 0 0] /CharProcs << /a {} 0 R /b {} 0 R >> /Encoding << /Type /Encoding /Differences [97 /a /b] >> /FirstChar 97 /LastChar 98 /Widths [{} {}] /ToUnicode {to_unicode} 0 R /Resources << >> >>",
            procs.0, procs.1, widths[0], widths[1]
        ))
    }

    const AB_TO_UNICODE: &str = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap 1 begincodespacerange <00> <FF> endcodespacerange 2 beginbfchar <61> <0061> <62> <0062> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";

    #[test]
    fn test_type3_size_and_widths_follow_the_font_matrix() {
        // Glyph space of 100 units per em: widths 50 and 60 are half and 0.6 of an em.
        let original = one_page(
            "<< /Font << /T3 5 0 R >> >>",
            "BT /T3 10 Tf 72 700 Td (ab) Tj ET",
            vec![
                type3_font("Boxes", [50, 60], 0.01, (6, 7), 8),
                stream_obj("", "50 0 d0 0 0 40 50 re f"),
                stream_obj("", "60 0 d0 0 0 50 60 re f"),
                stream_obj("", AB_TO_UNICODE),
            ],
        );
        let doc = extract(&original);
        let glyphs = visible_glyphs(&doc, doc.pages[0].glyphs[0].baseline_y);
        assert!((glyphs[0].font_size_pt - 10.0).abs() < 0.01, "size {}", glyphs[0].font_size_pt);
        assert!((glyphs[0].advance_pt - 5.0).abs() < 0.01 && (glyphs[1].advance_pt - 6.0).abs() < 0.01);
    }

    #[test]
    fn test_type3_glyphs_get_the_font_that_drew_them() {
        // Two Type 3 fonts with the same codes for the same characters but other widths, each
        // drawing a line; the second font again with 2 pt letter spacing.
        let original = one_page(
            "<< /Font << /T1 5 0 R /T2 6 0 R >> >>",
            "BT /T1 10 Tf 72 700 Td (ab) Tj ET BT /T2 10 Tf 72 650 Td (ab) Tj ET BT /T2 10 Tf 2 Tc 72 600 Td (ab) Tj ET",
            vec![
                type3_font("Narrow", [300, 300], 0.001, (7, 8), 9),
                type3_font("Wide", [500, 600], 0.001, (7, 8), 9),
                stream_obj("", "500 0 d0 0 0 400 500 re f"),
                stream_obj("", "600 0 d0 0 0 500 600 re f"),
                stream_obj("", AB_TO_UNICODE),
            ],
        );
        let doc = extract(&original);
        let top = doc.pages[0].glyphs[0].baseline_y;
        let lines = [(top, "Narrow", [3.0, 3.0]), (top + 50.0, "Wide", [5.0, 6.0]), (top + 100.0, "Wide", [5.0, 6.0])];
        for (baseline, name, advances) in lines {
            let glyphs = visible_glyphs(&doc, baseline);
            assert_eq!(doc.pages[0].fonts[glyphs[0].font_idx].postscript_name, name);
            assert!((glyphs[0].advance_pt - advances[0]).abs() < 0.01 && (glyphs[1].advance_pt - advances[1]).abs() < 0.01);
        }
    }

    #[test]
    fn test_type3_zero_width_mark_keeps_the_runs_font() {
        // `a`, `b` and a zero-width combining acute (code 99) in one run.
        let to_unicode = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap 1 begincodespacerange <00> <FF> endcodespacerange 3 beginbfchar <61> <0061> <62> <0062> <63> <0301> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
        let original = one_page(
            "<< /Font << /T3 5 0 R >> >>",
            "BT /T3 10 Tf 72 700 Td (abcab) Tj ET",
            vec![
                obj("<< /Type /Font /Subtype /Type3 /Name /Marks /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 6 0 R /b 7 0 R /c 8 0 R >> /Encoding << /Type /Encoding /Differences [97 /a /b /c] >> /FirstChar 97 /LastChar 99 /Widths [500 600 0] /ToUnicode 9 0 R /Resources << >> >>"),
                stream_obj("", "500 0 d0 0 0 400 500 re f"),
                stream_obj("", "600 0 d0 0 0 500 600 re f"),
                stream_obj("", "0 0 d0 -200 600 100 100 re f"),
                stream_obj("", to_unicode),
            ],
        );
        let doc = extract(&original);
        let page = &doc.pages[0];
        for g in page.glyphs.iter().filter(|g| g.unicode == "a" || g.unicode == "b") {
            assert_eq!(page.fonts[g.font_idx].postscript_name, "Marks", "{}", g.unicode);
        }
    }

    #[test]
    fn test_type3_font_is_reused_for_its_own_glyphs() {
        let to_unicode = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap 1 begincodespacerange <00> <FF> endcodespacerange 2 beginbfchar <61> <0061> <62> <0062> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
        let original = one_page(
            "<< /Font << /T3 5 0 R >> >>",
            "BT /T3 10 Tf 72 700 Td (ab) Tj ET",
            vec![
                obj("<< /Type /Font /Subtype /Type3 /Name /Squares /FontBBox [0 0 600 600] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 6 0 R /b 7 0 R >> /Encoding << /Type /Encoding /Differences [97 /a /b] >> /FirstChar 97 /LastChar 98 /Widths [500 600] /ToUnicode 8 0 R /Resources << >> >>"),
                stream_obj("", "500 0 d0 0 0 400 500 re f"),
                stream_obj("", "600 0 d0 0 0 500 600 re f"),
                stream_obj("", to_unicode),
            ],
        );
        let doc = extract(&original);
        let baseline = doc.pages[0].glyphs[0].baseline_y;
        let before = visible_glyphs(&doc, baseline);
        assert!((before[0].advance_pt - 5.0).abs() < 0.01, "Type 3 advances come from /Widths");
        let outcome = apply_text_edits(&original, &doc, &[edit_line(&doc, baseline, "ba")]).unwrap();
        let edited = extract(&outcome.pdf);
        let glyphs = visible_glyphs(&edited, baseline);
        assert_eq!(glyphs.iter().map(|g| g.unicode.as_str()).collect::<String>(), "ba");
        assert!((glyphs[0].advance_pt - 6.0).abs() < 0.01 && (glyphs[1].advance_pt - 5.0).abs() < 0.01);
        assert!((glyphs[1].bbox.x0 - 78.0).abs() < 0.01, "drawn with the Type 3 widths");
        let added = String::from_utf8_lossy(&outcome.pdf[original.len()..]).to_string();
        assert!(added.contains("/Subtype /Type3") && !added.contains("/BaseFont /Helvetica"));
    }
}
