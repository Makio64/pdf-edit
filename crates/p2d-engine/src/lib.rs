use std::collections::BTreeMap;
use std::sync::Arc;
use p2d_ir::*;
use p2d_layout::exact::reconstruct_exact;
use p2d_layout::{classify_page, reconstruct_document};
use p2d_ocr::{OcrWordBox, ocr_boxes_to_glyphs};
use p2d_pdf::{ExtractOptions, HayroBackend, OpenError, PdfBackend, decode_pdfium_binary_dump};
use p2d_writers::serialize_document;
use serde::{Deserialize, Serialize};

/// Quick inspection summary returned to the Chrome MV3 extension before or during conversion
/// (used to decide whether to show `"Remplir ce formulaire"` for AcroForms or prompt for a password).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfInspection {
    pub needs_password: bool,
    pub bad_password: bool,
    pub is_corrupt: bool,
    pub page_count: usize,
    pub has_acroform: bool,
    pub form_field_count: usize,
    pub primary_case: Option<PageCase>,
}

/// Resumable sans-IO state machine step for host-driven conversion.
#[derive(Debug, Clone)]
pub enum Step {
    NeedPassword,
    NeedOcr { page_index: usize, width_pt: f32, height_pt: f32 },
    Progress { completed_pages: usize, total_pages: usize },
    Done { output_bytes: Vec<u8>, flow_doc: FlowDoc, report: ConvertReport },
}

pub struct ConversionJob {
    pdf_bytes: Arc<[u8]>,
    password: Option<String>,
    params: Params,
    doc_ir: Option<DocIr>,
    cmap_recoveries: usize,
    pending_ocr_pages: Vec<usize>,
}

impl ConversionJob {
    pub fn new(pdf_bytes: Arc<[u8]>, password: Option<String>, params: Params) -> Self {
        Self {
            pdf_bytes,
            password,
            params,
            doc_ir: None,
            cmap_recoveries: 0,
            pending_ocr_pages: Vec::new(),
        }
    }

    pub fn provide_password(&mut self, pw: String) {
        self.password = Some(pw);
    }

    pub fn provide_ocr_boxes(&mut self, page_index: usize, boxes: &[OcrWordBox]) {
        if let Some(doc) = &mut self.doc_ir {
            if let Some(page) = doc.pages.get_mut(page_index) {
                page.glyphs.extend(ocr_boxes_to_glyphs(boxes));
            }
        }
    }

    pub fn step(&mut self) -> Result<Step, OpenError> {
        if self.doc_ir.is_none() {
            let backend = HayroBackend::new();
            let opts = extract_options_for(&self.params);
            match backend.extract_document(
                self.pdf_bytes.clone(),
                self.password.as_deref(),
                &opts,
            ) {
                Ok((doc, recoveries)) => {
                    self.pending_ocr_pages = doc
                        .pages
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, p)| {
                            if classify_page(p) == PageCase::ScannedPage {
                                Some(idx)
                            } else {
                                None
                            }
                        })
                        .collect();
                    self.cmap_recoveries = recoveries;
                    self.doc_ir = Some(doc);
                }
                Err(OpenError::NeedsPassword) => return Ok(Step::NeedPassword),
                Err(e) => return Err(e),
            }
        }

        if let Some(ocr_page_idx) = self.pending_ocr_pages.pop() {
            if let Some(doc) = &self.doc_ir {
                if let Some(p) = doc.pages.get(ocr_page_idx) {
                    return Ok(Step::NeedOcr {
                        page_index: ocr_page_idx,
                        width_pt: p.width_pt,
                        height_pt: p.height_pt,
                    });
                }
            }
        }

        let doc = self.doc_ir.as_ref().unwrap();
        let (flow_doc, mut report) = reconstruct(doc, &self.params);
        report.cmap_fallback_recoveries = self.cmap_recoveries;
        let output_bytes = serialize_document(&flow_doc, &self.params);

        Ok(Step::Done {
            output_bytes,
            flow_doc,
            report,
        })
    }
}

/// Background resolution of `Preset::Exact` pages (dpi).
pub const EXACT_BACKGROUND_DPI: f32 = 150.0;

fn extract_options_for(params: &Params) -> ExtractOptions {
    ExtractOptions {
        extract_images: params.preset != Preset::Exact,
        render_fallback_png: false,
        fallback_dpi: 150.0,
        render_graphics_dpi: (params.preset == Preset::Exact).then_some(EXACT_BACKGROUND_DPI),
    }
}

/// Lay out the document for the requested preset.
fn reconstruct(doc: &DocIr, params: &Params) -> (FlowDoc, ConvertReport) {
    if params.preset != Preset::Exact {
        return reconstruct_document(doc, params);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let metrics = p2d_layout::fonts::SystemFontMetrics::shared();
    #[cfg(target_arch = "wasm32")]
    let metrics = &p2d_layout::fonts::NoFontMetrics;
    reconstruct_exact(doc, params, metrics)
}

/// A PDF opened for direct editing: its extraction and its text as editable lines per page.
pub struct EditablePdf {
    pub pdf_bytes: Arc<[u8]>,
    pub doc: DocIr,
    pub pages: Vec<Vec<p2d_layout::exact::EditableLine>>,
    /// Sampled background colour behind each editable line (`pages[p][i]`).
    pub backgrounds: Vec<Vec<RgbaColor>>,
    /// Font programs embedded in the PDF, which the editor shows retyped text in.
    pub fonts: Vec<EditFont>,
    /// For each page, the `fonts` entry of each of its `PageIr::fonts` (`None`: not embedded).
    pub page_fonts: Vec<Vec<Option<usize>>>,
}

impl EditablePdf {
    /// The embedded font a run of a line is drawn in, as an index into `fonts`.
    pub fn run_font(&self, page_index: usize, run: &p2d_layout::exact::EditableRun) -> Option<usize> {
        *self.page_fonts.get(page_index)?.get(run.font_idx)?
    }
}

/// A font of the PDF, for the editor to load as a web font.
pub struct EditFont {
    /// CSS family name, one per distinct font.
    pub family: String,
    pub source: EditFontSource,
    pub bold: bool,
    pub italic: bool,
    /// The glyphs appear at this share of the size the PDF sets (`FontIr::size_scale`).
    pub size_scale: f32,
}

/// What an [`EditFont`] is built from.
pub enum EditFontSource {
    /// An embedded TrueType/OpenType program, with the characters the document draws with it
    /// (and their glyphs) and the advance the PDF sets for each glyph (1000 units per em).
    Program {
        program: FontProgram,
        chars: BTreeMap<char, u16>,
        widths: BTreeMap<u16, f32>,
    },
    /// The outlines of the glyphs the document draws, by character, for fonts a browser cannot
    /// load (Type 1, CFF, Type 3); the subsets a typeface is split into are merged.
    Shapes(BTreeMap<char, GlyphShape>),
}

impl EditFont {
    /// The font as a browser loads it; `None` when it cannot be built.
    pub fn web_font(&self) -> Option<Vec<u8>> {
        match &self.source {
            EditFontSource::Program { program, chars, widths } => {
                let chars: Vec<(char, u16)> = chars.iter().map(|(c, g)| (*c, *g)).collect();
                let widths: Vec<(u16, f32)> = widths.iter().map(|(g, w)| (*g, *w)).collect();
                p2d_pdf::sfnt::web_font(&program.0, &chars, &widths, &self.family, self.bold, self.italic)
            }
            EditFontSource::Shapes(shapes) => {
                let glyphs: Vec<(char, f32, &[PathOp])> =
                    shapes.values().map(|s| (s.unicode, s.advance, s.path.as_slice())).collect();
                p2d_pdf::sfnt::outline_font(&glyphs, &self.family, self.bold, self.italic)
            }
        }
    }
}

/// The fonts of `doc` the editor can show text in: distinct embedded programs a browser loads,
/// and the drawn outlines of the others (one per typeface), with what the document draws in each.
fn collect_edit_fonts(doc: &DocIr) -> (Vec<EditFont>, Vec<Vec<Option<usize>>>) {
    let mut fonts: Vec<EditFont> = Vec::new();
    // Typeface key of each shape font: name without subset tag, bold, italic.
    let mut shape_keys: Vec<(usize, (String, bool, bool))> = Vec::new();
    let mut page_fonts = Vec::with_capacity(doc.pages.len());
    for page in &doc.pages {
        let mut slots = Vec::with_capacity(page.fonts.len());
        for (font_idx, font) in page.fonts.iter().enumerate() {
            let program = font.program.as_ref().filter(|p| p2d_pdf::sfnt::is_sfnt(&p.0));
            let slot = if let Some(program) = program {
                let slot = fonts
                    .iter()
                    .position(|f| matches!(&f.source, EditFontSource::Program { program: p, .. } if p == program))
                    .unwrap_or_else(|| {
                        fonts.push(EditFont {
                            family: format!("p2d-font-{}", fonts.len() + 1),
                            source: EditFontSource::Program {
                                program: program.clone(),
                                chars: BTreeMap::new(),
                                widths: BTreeMap::new(),
                            },
                            bold: font.is_bold,
                            italic: font.is_italic,
                            size_scale: font.size_scale.unwrap_or(1.0),
                        });
                        fonts.len() - 1
                    });
                if let EditFontSource::Program { chars, widths, .. } = &mut fonts[slot].source {
                    for (gid, w) in &font.glyph_widths {
                        widths.entry(*gid).or_insert(*w);
                    }
                    for g in page.glyphs.iter().filter(|g| g.font_idx == font_idx) {
                        let mut cs = g.unicode.chars();
                        if let (Some(c), None) = (cs.next(), cs.next())
                            && c != '\u{FFFD}'
                        {
                            chars.entry(c).or_insert(g.glyph_id);
                        }
                    }
                }
                Some(slot)
            } else if !font.shapes.is_empty() {
                let key = (font.family_name.clone(), font.is_bold, font.is_italic);
                let slot = shape_keys.iter().find(|(_, k)| *k == key).map(|(i, _)| *i).unwrap_or_else(|| {
                    fonts.push(EditFont {
                        family: format!("p2d-font-{}", fonts.len() + 1),
                        source: EditFontSource::Shapes(BTreeMap::new()),
                        bold: font.is_bold,
                        italic: font.is_italic,
                        size_scale: 1.0,
                    });
                    shape_keys.push((fonts.len() - 1, key));
                    fonts.len() - 1
                });
                if let EditFontSource::Shapes(shapes) = &mut fonts[slot].source {
                    for shape in font.shapes.values() {
                        shapes.entry(shape.unicode).or_insert_with(|| shape.clone());
                    }
                }
                Some(slot)
            } else {
                None
            };
            slots.push(slot);
        }
        page_fonts.push(slots);
    }
    (fonts, page_fonts)
}

/// Open a PDF for direct editing (`p2d_pdf::edit`).
pub fn open_for_editing(pdf_bytes: &[u8], password: Option<&str>) -> Result<EditablePdf, OpenError> {
    let data: Arc<[u8]> = Arc::from(pdf_bytes);
    let opts = ExtractOptions {
        extract_images: false,
        render_fallback_png: false,
        fallback_dpi: 72.0,
        render_graphics_dpi: None,
    };
    let (doc, _) = HayroBackend::new().extract_document(data.clone(), password, &opts)?;
    let params = Params::default();
    #[cfg(not(target_arch = "wasm32"))]
    let metrics = p2d_layout::fonts::SystemFontMetrics::shared();
    #[cfg(target_arch = "wasm32")]
    let metrics = &p2d_layout::fonts::NoFontMetrics;
    let pages: Vec<Vec<p2d_layout::exact::EditableLine>> = doc
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            p2d_ir::progress::report(p2d_ir::progress::Step::Layout { page: i + 1, pages: doc.pages.len() });
            p2d_layout::exact::editable_lines(p, &params, metrics)
        })
        .collect();
    let page_boxes: Vec<Vec<RectPt>> = pages
        .iter()
        .map(|lines| {
            lines
                .iter()
                .map(|l| RectPt::new(l.bbox.x0 - 0.5, l.bbox.y0, l.bbox.x1 + 0.5, l.bbox.y1))
                .collect()
        })
        .collect();
    let backgrounds = p2d_pdf::sample_document_background_colors(data.clone(), Some(&doc), &page_boxes);
    let (fonts, page_fonts) = collect_edit_fonts(&doc);
    Ok(EditablePdf {
        pdf_bytes: data,
        doc,
        pages,
        backgrounds,
        fonts,
        page_fonts,
    })
}

/// New text for one editable line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineEdit {
    pub page_index: usize,
    pub line_id: usize,
    pub text: String,
    /// How `text` splits into the line's styles: index into `EditableLine::runs` and number of
    /// characters, in order. Empty: all of it in the line's first style.
    #[serde(default)]
    pub runs: Vec<(usize, usize)>,
}

/// The 14 fonts every PDF reader has (under their usual names), which PDFs often use without
/// embedding them.
fn is_standard_font(postscript_name: &str) -> bool {
    let base = postscript_name.split(['-', ',']).next().unwrap_or("");
    matches!(
        base,
        "Helvetica" | "Times" | "Courier" | "Symbol" | "ZapfDingbats" | "Arial" | "ArialMT" | "TimesNewRoman"
            | "TimesNewRomanPSMT" | "CourierNew" | "CourierNewPSMT"
    )
}

/// Advance (1000 units per em of the size the PDF sets) of each character of `text` in the font
/// of `run`: as the document draws it, else as the installed look-alike `face` does (standard
/// fonts have the same metrics everywhere).
fn char_widths(
    doc: &DocIr,
    page_index: usize,
    run: &p2d_layout::exact::EditableRun,
    text: &str,
    face: Option<&p2d_pdf::edit::InstalledFace>,
) -> Vec<(char, f32)> {
    let Some(page) = doc.pages.get(page_index) else {
        return Vec::new();
    };
    let scale = page.fonts.get(run.font_idx).and_then(|f| f.size_scale).unwrap_or(1.0);
    let mut out: Vec<(char, f32)> = Vec::new();
    for c in text.chars() {
        if out.iter().any(|(k, _)| *k == c) {
            continue;
        }
        let drawn = page.glyphs.iter().find(|g| {
            g.font_idx == run.font_idx && g.unicode.chars().eq(std::iter::once(c)) && g.font_size_pt > 0.0
        });
        let width = drawn
            .map(|g| g.advance_pt * scale / (g.font_size_pt * g.horiz_scale.max(0.05)) * 1000.0)
            .or_else(|| face.and_then(|f| p2d_pdf::sfnt::advance_1000(&f.data.0, f.index, c)));
        if let Some(w) = width {
            out.push((c, w));
        }
    }
    out
}

/// Characters the document draws in a font, which it can therefore draw again. Fonts are
/// recognised per page, so the font's own page is what counts.
fn known_chars(doc: &DocIr, page_index: usize, font_idx: usize) -> String {
    let mut chars: Vec<char> = doc
        .pages
        .get(page_index)
        .map(|p| {
            p.glyphs
                .iter()
                .filter(|g| g.font_idx == font_idx)
                .flat_map(|g| {
                    let mut cs = g.unicode.chars();
                    match (cs.next(), cs.next()) {
                        (Some(c), None) => Some(c),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    chars.sort_unstable();
    chars.dedup();
    chars.into_iter().collect()
}

/// The PDF with line edits written into it.
pub struct SavedEdits {
    pub pdf: Vec<u8>,
    /// How many lines had their old text covered rather than removed.
    pub covered: usize,
    /// How far the new text of the edited lines reaches across their page.
    pub reach: Vec<LineReach>,
}

/// Left and right edge of an edited line's new text, in visual points.
#[derive(Debug, Clone, Serialize)]
pub struct LineReach {
    pub page_index: usize,
    pub line_id: usize,
    pub x0: f32,
    pub x1: f32,
}

/// Write `edits` into the PDF.
pub fn save_line_edits(pdf: &EditablePdf, edits: &[LineEdit]) -> Result<SavedEdits, p2d_pdf::edit::EditError> {
    use p2d_pdf::edit::{InstalledFace, StandardFamily, TextEdit, TextPiece, apply_text_edits};
    #[cfg(not(target_arch = "wasm32"))]
    let metrics = p2d_layout::fonts::SystemFontMetrics::shared();
    let mut faces: Vec<((String, bool, bool), Option<InstalledFace>)> = Vec::new();
    // The installed face of each run's look-alike family, read once per family and style.
    let mut installed_face = |run: &p2d_layout::exact::EditableRun| -> Option<InstalledFace> {
        let key = (run.font_family.clone(), run.bold, run.italic);
        if let Some((_, face)) = faces.iter().find(|(k, _)| *k == key) {
            return face.clone();
        }
        #[cfg(not(target_arch = "wasm32"))]
        let face = metrics
            .face_file(&run.font_family, run.bold, run.italic)
            .or_else(|| p2d_layout::fonts::memory_face_file(&run.font_family, run.bold, run.italic))
            .map(|(data, index)| InstalledFace {
                data: FontProgram(Arc::from(data.as_slice())),
                index,
            });
        #[cfg(target_arch = "wasm32")]
        let face = p2d_layout::fonts::memory_face_file(&run.font_family, run.bold, run.italic)
            .map(|(data, index)| InstalledFace {
                data: FontProgram(Arc::from(data.as_slice())),
                index,
            });
        faces.push((key, face.clone()));
        face
    };

    let mut text_edits = Vec::new();
    // The line of each of `text_edits`.
    let mut edited_lines = Vec::new();
    for page_index in edits.iter().map(|e| e.page_index).collect::<std::collections::BTreeSet<_>>() {
        let lines = pdf
            .pages
            .get(page_index)
            .ok_or(p2d_pdf::edit::EditError::PageOutOfRange(page_index))?;
        let page_edits: Vec<(&LineEdit, &p2d_layout::exact::EditableLine)> = edits
            .iter()
            .filter(|e| e.page_index == page_index)
            .filter_map(|e| lines.get(e.line_id).map(|l| (e, l)))
            .filter(|(e, l)| e.text != l.text)
            .collect();
        let covers: Vec<RectPt> = page_edits
            .iter()
            .map(|(_, l)| RectPt::new(l.bbox.x0 - 0.5, l.bbox.y0, l.bbox.x1 + 0.5, l.bbox.y1))
            .collect();
        let cached_bg = pdf.backgrounds.get(page_index);
        let colors: Vec<RgbaColor> = if let Some(bgs) = cached_bg
            && page_edits.iter().all(|(e, _)| e.line_id < bgs.len())
        {
            page_edits.iter().map(|(e, _)| bgs[e.line_id]).collect()
        } else {
            p2d_pdf::sample_background_colors(pdf.pdf_bytes.clone(), page_index, &covers)
        };
        for (((e, l), cover), color) in page_edits.into_iter().zip(covers).zip(colors) {
            let spans = if e.runs.is_empty() {
                vec![(0, e.text.chars().count())]
            } else {
                e.runs.clone()
            };
            let mut chars = e.text.chars();
            let pieces = spans
                .iter()
                .filter_map(|&(run_index, len)| {
                    let text: String = chars.by_ref().take(len).collect();
                    let run = l.runs.get(run_index).or(l.runs.first())?;
                    let font = pdf.doc.pages.get(page_index).and_then(|p| p.fonts.get(run.font_idx));
                    // A standard font the PDF does not embed is drawn by each reader its own way:
                    // keep the same standard font for the new text rather than an installed one.
                    let standard_only =
                        font.is_some_and(|f| f.program.is_none() && is_standard_font(&f.postscript_name));
                    let fallback = installed_face(run);
                    let widths = char_widths(&pdf.doc, page_index, run, &text, fallback.as_ref());
                    Some(TextPiece {
                        text,
                        glyph_index: Some(run.glyph_index),
                        font_size_pt: run.font_size_pt,
                        nominal_font_size_pt: run.nominal_font_size_pt,
                        color: run.color,
                        family: if run.monospace {
                            StandardFamily::Courier
                        } else if run.serif {
                            StandardFamily::Times
                        } else {
                            StandardFamily::Helvetica
                        },
                        bold: run.bold,
                        italic: run.italic,
                        fallback: if standard_only { None } else { fallback.clone() },
                        fallback_same_family: run.same_family,
                        known_chars: known_chars(&pdf.doc, page_index, run.font_idx),
                        char_widths: widths,
                        other_fonts: run
                            .other_glyphs
                            .iter()
                            .map(|&g| {
                                let font_idx = pdf.doc.pages[page_index].glyphs.get(g).map_or(0, |g| g.font_idx);
                                (g, known_chars(&pdf.doc, page_index, font_idx))
                            })
                            .collect(),
                    })
                })
                .collect();
            edited_lines.push((page_index, e.line_id));
            text_edits.push(TextEdit {
                page_index,
                glyph_indices: l.glyph_indices.clone(),
                pieces,
                x_pt: l.bbox.x0,
                baseline_pt: l.baseline,
                x1_pt: l.bbox.x1,
                align: l.align,
                char_spacing_pt: l.char_spacing_pt,
                word_spacing_pt: l.word_spacing_pt,
                horiz_scale: l.horiz_scale,
                cover_bbox: cover,
                cover_color: color,
            });
        }
    }
    let outcome = apply_text_edits(&pdf.pdf_bytes, &pdf.doc, &text_edits)?;
    let reach = edited_lines
        .into_iter()
        .zip(outcome.extents)
        .filter_map(|((page_index, line_id), extent)| {
            extent.map(|(x0, x1)| LineReach { page_index, line_id, x0, x1 })
        })
        .collect();
    Ok(SavedEdits { pdf: outcome.pdf, covered: outcome.covered_edits, reach })
}

/// Inspect a PDF quickly (used by the Gmail content script / worker to detect AcroForms or password prompts).
pub fn inspect_pdf_bytes(pdf_bytes: &[u8], password: Option<&str>) -> PdfInspection {
    let backend = HayroBackend::new();
    let opts = ExtractOptions {
        extract_images: false,
        render_fallback_png: false,
        fallback_dpi: 72.0,
        render_graphics_dpi: None,
    };
    match backend.extract_document(Arc::from(pdf_bytes), password, &opts) {
        Ok((doc, _)) => {
            let form_field_count = doc.pages.iter().map(|p| p.form_fields.len()).sum();
            let primary_case = doc.pages.first().map(classify_page);
            PdfInspection {
                needs_password: false,
                bad_password: false,
                is_corrupt: false,
                page_count: doc.pages.len(),
                has_acroform: doc.has_acroform || form_field_count > 0,
                form_field_count,
                primary_case,
            }
        }
        Err(OpenError::NeedsPassword) => PdfInspection {
            needs_password: true,
            bad_password: false,
            is_corrupt: false,
            page_count: 0,
            has_acroform: false,
            form_field_count: 0,
            primary_case: None,
        },
        Err(OpenError::BadPassword) => PdfInspection {
            needs_password: true,
            bad_password: true,
            is_corrupt: false,
            page_count: 0,
            has_acroform: false,
            form_field_count: 0,
            primary_case: None,
        },
        Err(_) => PdfInspection {
            needs_password: false,
            bad_password: false,
            is_corrupt: true,
            page_count: 0,
            has_acroform: false,
            form_field_count: 0,
            primary_case: None,
        },
    }
}

/// Convert a PDF buffer (or a `P2DDMP01` binary page dump) to the target format with the Never-Fail Ladder.
/// Guarantees that a valid output file is always produced even if the input PDF is malformed.
pub fn convert_pdf_never_fail(
    input_bytes: &[u8],
    password: Option<&str>,
    params: &Params,
) -> Result<(Vec<u8>, FlowDoc, ConvertReport), OpenError> {
    // Check if input is a zero-copy PDFium binary dump (`P2DDMP01`)
    if input_bytes.starts_with(p2d_pdf::PDFIUM_DUMP_MAGIC) {
        let doc_ir = decode_pdfium_binary_dump(input_bytes)?;
        let (flow_doc, mut report) = reconstruct_document(&doc_ir, params);
        for pr in &mut report.pages {
            pr.rung_used = FallbackRung::SecondaryBackend;
        }
        let out = serialize_document(&flow_doc, params);
        return Ok((out, flow_doc, report));
    }

    let backend = HayroBackend::new();
    let opts = extract_options_for(params);

    let arc_bytes: Arc<[u8]> = Arc::from(input_bytes);
    match backend.extract_document(arc_bytes, password, &opts) {
        Ok((doc_ir, cmap_recoveries)) => {
            let (flow_doc, mut report) = reconstruct(&doc_ir, params);
            report.cmap_fallback_recoveries = cmap_recoveries;
            p2d_ir::progress::report(p2d_ir::progress::Step::Write);
            let out = serialize_document(&flow_doc, params);
            Ok((out, flow_doc, report))
        }
        Err(OpenError::NeedsPassword) => Err(OpenError::NeedsPassword),
        Err(OpenError::BadPassword) => Err(OpenError::BadPassword),
        Err(_corrupt_err) => {
            // Never-Fail Ladder Rung 5: always return a valid document explaining the recovery
            let fallback_para = Paragraph {
                runs: vec![TextRun {
                    text: "Ce document PDF comportait une structure endommagée et a été ouvert en mode de secours.".to_string(),
                    style: RunStyle::default(),
                }],
                ..Default::default()
            };
            let flow_doc = FlowDoc {
                title: Some("Document récupéré".to_string()),
                default_font_family: "Helvetica Neue".to_string(),
                default_font_size_pt: 11.0,
                sections: vec![Section {
                    blocks: vec![FlowBlock::Paragraph(fallback_para)],
                    ..Default::default()
                }],
                source_pages: Vec::new(),
            };
            let report = ConvertReport {
                page_count: 1,
                pages: vec![PageReport {
                    page_index: 0,
                    detected_case: PageCase::DigitalLetter,
                    rung_used: FallbackRung::PageImagePlusText,
                    glyph_count: 0,
                    word_count: 16,
                    table_count: 0,
                    unmapped_glyph_ratio: 0.0,
                    warnings: vec!["Recovered via Never-Fail Rung 5".to_string()],
                }],
                total_words: 16,
                ..Default::default()
            };
            let out = serialize_document(&flow_doc, params);
            Ok((out, flow_doc, report))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_never_fail_ladder_on_corrupt_bytes() {
        let bad_pdf = b"not a valid pdf at all";
        let (docx, _flow, report) =
            convert_pdf_never_fail(bad_pdf, None, &Params::default()).expect("Never-fail must succeed");
        assert!(docx.starts_with(b"PK\x03\x04"));
        assert_eq!(report.pages[0].rung_used, FallbackRung::PageImagePlusText);
    }
}
