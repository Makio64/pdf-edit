pub mod edit;
pub mod sfnt;
mod tounicode;

use std::collections::BTreeMap;
use std::sync::Arc;

use hayro::{PixmapSettings, RenderCache, RenderSettings};
use hayro_interpret::font::{Glyph, GlyphRun};
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::hayro_syntax::content::ops::TypedInstruction;
use hayro_interpret::hayro_syntax::object::dict::keys::*;
use hayro_interpret::hayro_syntax::object::{Array, Dict, Name, Rect as PdfRect, String as PdfString};
use hayro_interpret::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageData, ImageDrawProps,
    InterpreterCache, InterpreterSettings, MarkedContentProperties, Paint, SoftMask, TransformExt,
    interpret_page,
};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};
use p2d_ir::*;
use p2d_ir::progress::Step;
use skrifa::MetadataProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    NeedsPassword,
    BadPassword,
    Corrupt(String),
    Unsupported(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsPassword => write!(f, "PDF requires a password"),
            Self::BadPassword => write!(f, "Invalid PDF password"),
            Self::Corrupt(msg) => write!(f, "Corrupt PDF: {msg}"),
            Self::Unsupported(msg) => write!(f, "Unsupported PDF feature: {msg}"),
        }
    }
}

impl std::error::Error for OpenError {}

#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    pub extract_images: bool,
    pub render_fallback_png: bool,
    pub fallback_dpi: f32,
    /// When set, render each page without its horizontal text at this resolution into
    /// `PageIr::graphics_png` (background of `Preset::Exact` documents).
    pub render_graphics_dpi: Option<f32>,
}

impl ExtractOptions {
    pub fn full() -> Self {
        Self {
            extract_images: true,
            render_fallback_png: false,
            fallback_dpi: 150.0,
            render_graphics_dpi: None,
        }
    }
}

pub trait PdfBackend {
    fn name(&self) -> &'static str;
    fn extract_document(
        &self,
        data: Arc<[u8]>,
        password: Option<&str>,
        opts: &ExtractOptions,
    ) -> Result<(DocIr, usize), OpenError>;
    fn render_page_png(
        &self,
        data: Arc<[u8]>,
        password: Option<&str>,
        page_index: usize,
        dpi: f32,
    ) -> Result<Vec<u8>, OpenError>;
}

/// Primary pure-Rust backend built on `hayro` + `skrifa` embedded-font `cmap`/`post` fallback.
#[derive(Debug, Default, Clone)]
pub struct HayroBackend;

impl HayroBackend {
    pub fn new() -> Self {
        Self
    }
}

impl PdfBackend for HayroBackend {
    fn name(&self) -> &'static str {
        "hayro"
    }

    fn extract_document(
        &self,
        data: Arc<[u8]>,
        password: Option<&str>,
        opts: &ExtractOptions,
    ) -> Result<(DocIr, usize), OpenError> {
        let pdf = open_hayro_pdf(data, password)?;
        let cache = InterpreterCache::new();
        let settings = InterpreterSettings::default();
        let render_cache = RenderCache::new();

        let mut pages_ir = Vec::with_capacity(pdf.pages().len());
        let mut total_cmap_recoveries = 0usize;
        // Each embedded font program and its decoded tables, read once for all the pages that use it.
        let mut programs: BTreeMap<u128, FontProgram> = BTreeMap::new();
        let mut font_cmap_cache: BTreeMap<u128, InvertedFontMap> = BTreeMap::new();
        let mut font_scale_cache: BTreeMap<u128, f32> = BTreeMap::new();
        let mut empty_glyph_cache: BTreeMap<(u128, u32), bool> = BTreeMap::new();
        let mut has_acroform = false;

        // Check document-level trailer/root for AcroForm and collect Type1 /BaseFont width tables
        let mut pdf_font_tables: Vec<(String, u32, Vec<i32>)> = Vec::new();
        for obj in pdf.objects() {
            if let Some(dict) = obj.into_dict() {
                if dict.contains_key(ACRO_FORM) {
                    has_acroform = true;
                }
                if let Some(bf) = dict.get::<Name<'_>>(BASE_FONT) {
                    if let (Some(fc), Some(widths_arr)) = (
                        dict.get::<i32>(FIRST_CHAR),
                        dict.get::<Array<'_>>(WIDTHS),
                    ) {
                        let base_name = String::from_utf8_lossy(bf.as_ref()).to_string();
                        let widths: Vec<i32> = widths_arr
                            .iter::<f32>()
                            .map(|w| w.round() as i32)
                            .collect();
                        if !widths.is_empty() && fc >= 0 {
                            pdf_font_tables.push((base_name, fc as u32, widths));
                        }
                    }
                }
            }
        }
        let pdf_font_tables: Arc<[(String, u32, Vec<i32>)]> = pdf_font_tables.into();

        let page_count = pdf.pages().len();
        for (page_index, page) in pdf.pages().iter().enumerate() {
            p2d_ir::progress::report(Step::Read { page: page_index + 1, pages: page_count });
            let (width_pt, height_pt) = page.render_dimensions();
            let rotation_deg = match page.rotation() {
                hayro_interpret::hayro_syntax::page::Rotation::None => 0,
                hayro_interpret::hayro_syntax::page::Rotation::Horizontal => 90,
                hayro_interpret::hayro_syntax::page::Rotation::Flipped => 180,
                hayro_interpret::hayro_syntax::page::Rotation::FlippedHorizontal => 270,
            };

            let init_tf = page.initial_transform(true).to_kurbo();
            let page_bbox = Rect::new(0.0, 0.0, width_pt as f64, height_pt as f64);

            // Pre-scan content stream for active Tc (char spacing) and Tw (word spacing) hints
            let (default_tc, default_tw) = scan_page_spacing_hints(page);

            let mut state = Context::new(
                init_tf,
                page_bbox,
                &cache,
                page.xref(),
                settings.clone(),
            );

            let mut device = HayroExtractDevice::new(
                page_index,
                width_pt,
                height_pt,
                opts.extract_images,
                default_tc,
                default_tw,
                pdf_font_tables.clone(),
            );
            device.type3_fonts = page_type3_fonts(page.resources());
            device.programs = std::mem::take(&mut programs);
            device.font_cmap_cache = std::mem::take(&mut font_cmap_cache);
            device.font_scale_cache = std::mem::take(&mut font_scale_cache);
            device.empty_glyph_cache = std::mem::take(&mut empty_glyph_cache);

            interpret_page(page, &mut state, &mut device);
            programs = std::mem::take(&mut device.programs);
            font_cmap_cache = std::mem::take(&mut device.font_cmap_cache);
            font_scale_cache = std::mem::take(&mut device.font_scale_cache);
            empty_glyph_cache = std::mem::take(&mut device.empty_glyph_cache);

            // Extract annotations and AcroForm widget fields on this page
            let (annots, fields) = extract_page_annotations(page, page_index, init_tf);
            if !fields.is_empty() {
                has_acroform = true;
            }

            let has_visible_glyphs = device
                .glyphs
                .iter()
                .any(|g| g.is_visible_in_clip && g.render_mode != RenderMode::Invisible && !g.unicode.trim().is_empty());
            let needs_raster_fallback = opts.render_fallback_png
                || !has_visible_glyphs
                || (device.glyphs.len() < 15
                    && (device.paths.len() >= 20
                        || device.images.iter().any(|img| img.is_full_page_scan)));

            let has_complex_graphics = device.complex_graphics
                || device.glyphs.iter().any(|g| g.angle_deg.abs() > TEXT_SKIP_MAX_ANGLE_DEG);

            total_cmap_recoveries += device.cmap_fallback_recoveries;

            pages_ir.push(PageIr {
                page_index,
                width_pt,
                height_pt,
                rotation_deg,
                fonts: device.fonts,
                glyphs: device.glyphs,
                paths: device.paths,
                images: device.images,
                annotations: annots,
                form_fields: fields,
                page_raster_png: needs_raster_fallback.then(Vec::new),
                graphics_png: None,
                graphics_format: String::new(),
                has_complex_graphics,
            });
        }

        let pdf_pages = pdf.pages();
        let render_count = pages_ir
            .iter()
            .filter(|p| p.page_raster_png.is_some() || (opts.render_graphics_dpi.is_some() && p.has_complex_graphics))
            .count();

        // The worker threads report to this thread's progress sink.
        let sink = p2d_ir::progress::current().filter(|_| render_count > 0);
        let drawn = std::sync::atomic::AtomicUsize::new(0);
        let page_drawn = || {
            let done = drawn.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if let Some(sink) = &sink {
                sink(Step::Draw { done, pages: page_count });
            }
        };
        if let Some(sink) = &sink {
            sink(Step::Draw { done: 0, pages: page_count });
        }

        #[cfg(not(target_arch = "wasm32"))]
        if render_count > 1 {
            let workers = std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .min(pages_ir.len());
            let chunk_size = pages_ir.len().div_ceil(workers);
            std::thread::scope(|s| {
                for chunk in pages_ir.chunks_mut(chunk_size) {
                    let settings = &settings;
                    let page_drawn = &page_drawn;
                    s.spawn(move || {
                        let rc = RenderCache::new();
                        for page_ir in chunk {
                            if let Some(page) = pdf_pages.get(page_ir.page_index) {
                                render_page_rasters(page, page_ir, &rc, settings, opts);
                            }
                            page_drawn();
                        }
                    });
                }
            });
        } else {
            for page_ir in &mut pages_ir {
                if let Some(page) = pdf_pages.get(page_ir.page_index) {
                    render_page_rasters(page, page_ir, &render_cache, &settings, opts);
                }
                page_drawn();
            }
        }

        #[cfg(target_arch = "wasm32")]
        for page_ir in &mut pages_ir {
            if let Some(page) = pdf_pages.get(page_ir.page_index) {
                render_page_rasters(page, page_ir, &render_cache, &settings, opts);
            }
            page_drawn();
        }

        let doc = DocIr {
            pages: pages_ir,
            has_acroform,
            producer: None,
            title: None,
        };

        Ok((doc, total_cmap_recoveries))
    }

    fn render_page_png(
        &self,
        data: Arc<[u8]>,
        password: Option<&str>,
        page_index: usize,
        dpi: f32,
    ) -> Result<Vec<u8>, OpenError> {
        let pdf = open_hayro_pdf(data, password)?;
        let page = pdf
            .pages()
            .get(page_index)
            .ok_or_else(|| OpenError::Corrupt(format!("Page {page_index} out of bounds")))?;
        let render_cache = RenderCache::new();
        let settings = InterpreterSettings::default();
        let scale = (dpi / 72.0).clamp(0.5, 4.0);
        let pixmap = hayro::render(
            page,
            &render_cache,
            &settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            },
        );
        encode_rgba_png(
            pixmap.width() as u32,
            pixmap.height() as u32,
            pixmap.data_as_u8_slice(),
        )
        .map_err(|e| OpenError::Corrupt(format!("PNG encode error: {e}")))
    }
}

/// Glyph runs rotated by at most this many degrees are re-created as live text in `Preset::Exact`
/// documents, so they are left out of the page background; more rotated text stays in the image.
pub const TEXT_SKIP_MAX_ANGLE_DEG: f32 = 2.0;

fn render_page_rasters<'a>(
    page: &'a hayro_interpret::hayro_syntax::page::Page<'a>,
    page_ir: &mut PageIr,
    rc: &RenderCache<'a>,
    settings: &InterpreterSettings,
    opts: &ExtractOptions,
) {
    if page_ir.page_raster_png.is_some() {
        let scale = (opts.fallback_dpi / 72.0).clamp(0.5, 4.0);
        let pixmap = hayro::render(
            page,
            rc,
            settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
            },
        );
        page_ir.page_raster_png = encode_rgba_png(
            pixmap.width() as u32,
            pixmap.height() as u32,
            pixmap.data_as_u8_slice(),
        )
        .ok();
    }
    if let Some(dpi) = opts.render_graphics_dpi
        && page_ir.has_complex_graphics
        && let Some((bytes, fmt)) = render_graphics_background(page, rc, settings, dpi)
    {
        page_ir.graphics_png = Some(bytes);
        page_ir.graphics_format = fmt.to_string();
    }
}

/// Render `page` without its (near-)horizontal text. Returns `None` for an all-white page, else the
/// encoded image and its format (`"png"`, or `"jpeg"` when that is much smaller, e.g. for photos).
fn render_graphics_background<'a>(
    page: &'a hayro_interpret::hayro_syntax::page::Page<'a>,
    render_cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    dpi: f32,
) -> Option<(Vec<u8>, &'static str)> {
    let scale = (dpi / 72.0).clamp(0.5, 4.0);
    let pixmap = hayro::render(
        page,
        render_cache,
        settings,
        &RenderSettings {
            skip_text: true,
            skip_text_max_angle_deg: TEXT_SKIP_MAX_ANGLE_DEG,
            ..Default::default()
        },
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    let rgba = pixmap.data_as_u8_slice();
    if rgba.chunks_exact(4).all(|px| px[0] >= 253 && px[1] >= 253 && px[2] >= 253) {
        return None;
    }
    let (w, h) = (pixmap.width() as u32, pixmap.height() as u32);
    let png = encode_rgba_png(w, h, rgba).ok()?;
    if png.len() < 300_000 {
        return Some((png, "png"));
    }
    let mut jpeg = Vec::new();
    let encoded = jpeg_encoder::Encoder::new(&mut jpeg, 90)
        .encode(rgba, w as u16, h as u16, jpeg_encoder::ColorType::Rgba)
        .is_ok();
    if encoded && jpeg.len() * 2 < png.len() {
        Some((jpeg, "jpeg"))
    } else {
        Some((png, "png"))
    }
}

/// Colour of the page under each rectangle (visual page points), ignoring text: the median of the
/// pixels of a 72 dpi render without text. Used to hide text that cannot be removed.
pub fn sample_background_colors(data: Arc<[u8]>, page_index: usize, rects: &[RectPt]) -> Vec<RgbaColor> {
    if rects.is_empty() {
        return Vec::new();
    }
    let white = vec![RgbaColor::WHITE; rects.len()];
    let Ok(pdf) = open_hayro_pdf(data, None) else {
        return white;
    };
    let Some(page) = pdf.pages().get(page_index) else {
        return white;
    };
    sample_page_rects(page, &RenderCache::new(), &InterpreterSettings::default(), rects)
}

/// Colour of each page under its rectangles, opening the PDF at most once and skipping the 72 dpi
/// render on pages whose extracted IR has no non-white graphics or images.
pub fn sample_document_background_colors(
    data: Arc<[u8]>,
    doc: Option<&DocIr>,
    page_rects: &[Vec<RectPt>],
) -> Vec<Vec<RgbaColor>> {
    let mut out: Vec<Vec<RgbaColor>> = page_rects
        .iter()
        .map(|rects| vec![RgbaColor::WHITE; rects.len()])
        .collect();
    let needs_render = |page_index: usize, rects: &[RectPt]| -> bool {
        if rects.is_empty() {
            return false;
        }
        let Some(page) = doc.and_then(|d| d.pages.get(page_index)) else {
            return true;
        };
        if page.has_complex_graphics || !page.images.is_empty() {
            return true;
        }
        let is_white = |c: RgbaColor| c.a == 0 || (c.r >= 253 && c.g >= 253 && c.b >= 253);
        page.paths
            .iter()
            .any(|p| !p.fill_color.is_none_or(is_white) || !p.stroke_color.is_none_or(is_white))
    };
    if !page_rects.iter().enumerate().any(|(i, r)| needs_render(i, r)) {
        return out;
    }
    let Ok(pdf) = open_hayro_pdf(data, None) else {
        return out;
    };
    let render_cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    for (page_index, rects) in page_rects.iter().enumerate() {
        p2d_ir::progress::report(Step::Colors { page: page_index + 1, pages: page_rects.len() });
        if !needs_render(page_index, rects) {
            continue;
        }
        if let Some(page) = pdf.pages().get(page_index) {
            out[page_index] = sample_page_rects(page, &render_cache, &settings, rects);
        }
    }
    out
}

fn sample_page_rects<'a>(
    page: &'a hayro_interpret::hayro_syntax::page::Page<'a>,
    render_cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    rects: &[RectPt],
) -> Vec<RgbaColor> {
    let pixmap = hayro::render(
        page,
        render_cache,
        settings,
        &RenderSettings {
            skip_text: true,
            skip_text_max_angle_deg: TEXT_SKIP_MAX_ANGLE_DEG,
            ..Default::default()
        },
        &PixmapSettings {
            x_scale: 1.0,
            y_scale: 1.0,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    let (w, h) = (pixmap.width() as i64, pixmap.height() as i64);
    let px = pixmap.data_as_u8_slice();
    rects
        .iter()
        .map(|r| {
            let mut samples: Vec<[u8; 3]> = Vec::new();
            for y in (r.y0.floor() as i64).max(0)..(r.y1.ceil() as i64).min(h) {
                for x in (r.x0.floor() as i64).max(0)..(r.x1.ceil() as i64).min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    samples.push([px[i], px[i + 1], px[i + 2]]);
                }
            }
            if samples.is_empty() {
                return RgbaColor::WHITE;
            }
            samples.sort_by_key(|c| c[0] as u32 + c[1] as u32 + c[2] as u32);
            let [r, g, b] = samples[samples.len() / 2];
            RgbaColor { r, g, b, a: 255 }
        })
        .collect()
}

fn open_hayro_pdf(data: Arc<[u8]>, password: Option<&str>) -> Result<Pdf, OpenError> {
    let pw = password.unwrap_or("");
    match Pdf::new_with_password(Arc::new(data), pw) {
        Ok(pdf) => Ok(pdf),
        Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => {
            if pw.is_empty() {
                Err(OpenError::NeedsPassword)
            } else {
                Err(OpenError::BadPassword)
            }
        }
        Err(LoadPdfError::Decryption(err)) => {
            Err(OpenError::Unsupported(format!("Decryption error: {err:?}")))
        }
        Err(LoadPdfError::Invalid) => Err(OpenError::Corrupt("Invalid PDF structure".to_string())),
    }
}

/// Scan page operators to capture character spacing (`Tc`) and word spacing (`Tw`) values per text block.
fn scan_page_spacing_hints(page: &hayro_interpret::hayro_syntax::page::Page<'_>) -> (f32, f32) {
    let mut last_tc = 0.0f32;
    let mut last_tw = 0.0f32;
    let mut ops = page.typed_operations();
    while let Some(op) = ops.next() {
        match op {
            TypedInstruction::CharacterSpacing(tc) => {
                if tc.0.as_f32().abs() > last_tc.abs() {
                    last_tc = tc.0.as_f32();
                }
            }
            TypedInstruction::WordSpacing(tw) => {
                if tw.0.as_f32().abs() > last_tw.abs() {
                    last_tw = tw.0.as_f32();
                }
            }
            _ => {}
        }
    }
    (last_tc, last_tw)
}

fn decode_pdf_string(s: &PdfString<'_>) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let utf16: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&utf16)
    } else if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let utf16: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&utf16)
    } else if let Ok(utf8) = std::str::from_utf8(bytes) {
        utf8.to_string()
    } else {
        // PDFDocEncoding / WinAnsi fallback for Latin-1
        bytes.iter().map(|&b| b as char).collect()
    }
}

fn extract_page_annotations(
    page: &hayro_interpret::hayro_syntax::page::Page<'_>,
    page_index: usize,
    init_tf: Affine,
) -> (Vec<AnnotIr>, Vec<FormFieldIr>) {
    let mut annots = Vec::new();
    let mut fields = Vec::new();

    let Some(annot_array) = page.raw().get::<Array<'_>>(ANNOTS) else {
        return (annots, fields);
    };

    for dict in annot_array.iter::<Dict<'_>>() {
        let subtype = dict
            .get::<Name<'_>>(SUBTYPE)
            .map(|n| n.as_str().to_string())
            .unwrap_or_default();

        let bbox = dict
            .get::<PdfRect>(RECT)
            .map(|r| transform_rect(init_tf, Rect::new(r.x0, r.y0, r.x1, r.y1)))
            .unwrap_or_default();

        let contents = dict
            .get::<PdfString<'_>>(CONTENTS)
            .map(|s| decode_pdf_string(&s))
            .filter(|s| !s.trim().is_empty());

        match subtype.as_str() {
            "Widget" => {
                let ft = dict
                    .get::<Name<'_>>(FT)
                    .map(|n| n.as_str().to_string())
                    .unwrap_or_else(|| "Tx".to_string());
                let field_name = dict
                    .get::<PdfString<'_>>(T)
                    .map(|s| decode_pdf_string(&s))
                    .unwrap_or_else(|| format!("field_{}", fields.len()));
                let val_str = dict
                    .get::<PdfString<'_>>(V)
                    .map(|s| decode_pdf_string(&s))
                    .or_else(|| {
                        dict.get::<Name<'_>>(V)
                            .map(|n| n.as_str().to_string())
                    });

                let kind = match ft.as_str() {
                    "Btn" => {
                        let checked = val_str
                            .as_deref()
                            .is_some_and(|v| v != "Off" && !v.is_empty());
                        FieldKind::CheckBox { checked }
                    }
                    "Ch" => FieldKind::Choice,
                    "Sig" => FieldKind::Signature,
                    _ => FieldKind::Text,
                };
                fields.push(FormFieldIr {
                    name: field_name,
                    kind,
                    value: val_str.or(contents),
                    bbox,
                    page_index,
                });
            }
            "FreeText" => {
                annots.push(AnnotIr {
                    kind: AnnotKind::FreeText,
                    bbox,
                    text_contents: contents,
                    color: Some(RgbaColor::BLACK),
                    font_size_pt: Some(11.0),
                    appearance_png: None,
                });
            }
            "Highlight" => {
                annots.push(AnnotIr {
                    kind: AnnotKind::Highlight,
                    bbox,
                    text_contents: contents,
                    color: Some(RgbaColor {
                        r: 255,
                        g: 245,
                        b: 120,
                        a: 255,
                    }),
                    font_size_pt: None,
                    appearance_png: None,
                });
            }
            "Ink" => {
                annots.push(AnnotIr {
                    kind: AnnotKind::Ink,
                    bbox,
                    text_contents: contents,
                    color: None,
                    font_size_pt: None,
                    appearance_png: None,
                });
            }
            "Stamp" => {
                annots.push(AnnotIr {
                    kind: AnnotKind::Stamp,
                    bbox,
                    text_contents: contents,
                    color: None,
                    font_size_pt: None,
                    appearance_png: None,
                });
            }
            "Link" => {
                let uri = dict
                    .get::<Dict<'_>>(A)
                    .and_then(|a| a.get::<PdfString<'_>>(URI))
                    .map(|s| decode_pdf_string(&s));
                if let Some(uri) = uri {
                    annots.push(AnnotIr {
                        kind: AnnotKind::Link { uri },
                        bbox,
                        text_contents: None,
                        color: None,
                        font_size_pt: None,
                        appearance_png: None,
                    });
                }
            }
            _ => {}
        }
    }

    (annots, fields)
}

/// Inverted `GlyphId -> String` map extracted from an embedded font's `cmap` and `post` tables via `skrifa`.
#[derive(Debug, Default)]
struct InvertedFontMap {
    gid_to_unicode: BTreeMap<u16, String>,
}

impl InvertedFontMap {
    fn from_font_bytes(bytes: &[u8]) -> Self {
        let mut map = BTreeMap::new();
        if let Ok(font_ref) = skrifa::FontRef::from_index(bytes, 0) {
            // 1. Invert the embedded TrueType/OpenType `cmap` table
            let charmap = font_ref.charmap();
            for (codepoint, gid) in charmap.mappings() {
                if let Some(ch) = char::from_u32(codepoint) {
                    let gid_u16 = gid.to_u32() as u16;
                    map.entry(gid_u16).or_insert_with(|| ch.to_string());
                }
            }

            // 2. Fallback to `post` / CFF glyph names (e.g. `uni00E9`, `eacute`, `space`)
            let glyph_names = font_ref.glyph_names();
            let num_glyphs = glyph_names.num_glyphs().min(2048);
            for gid_u32 in 0..num_glyphs {
                let gid_u16 = gid_u32 as u16;
                if map.contains_key(&gid_u16) {
                    continue;
                }
                if let Some(name) = glyph_names.get(skrifa::GlyphId::from(gid_u16)) {
                    if let Some(decoded) = resolve_postscript_glyph_name(name.as_str()) {
                        map.insert(gid_u16, decoded);
                    }
                }
            }
        }
        Self {
            gid_to_unicode: map,
        }
    }
}

fn resolve_postscript_glyph_name(name: &str) -> Option<String> {
    let base = name.split('.').next().unwrap_or(name);
    if base == ".notdef" || base == ".null" {
        return None;
    }
    if let Some(hex) = base.strip_prefix("uni") {
        if hex.len() == 4 {
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                return char::from_u32(cp).map(|c| c.to_string());
            }
        }
    }
    if let Some(hex) = base.strip_prefix('u') {
        if (4..=6).contains(&hex.len()) {
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                return char::from_u32(cp).map(|c| c.to_string());
            }
        }
    }
    let ch = match base {
        "space" | "nbspace" => " ",
        "exclam" => "!",
        "quotedbl" => "\"",
        "numbersign" => "#",
        "dollar" => "$",
        "percent" => "%",
        "ampersand" => "&",
        "quotesingle" | "quoteright" => "’",
        "quoteleft" => "‘",
        "quotedblleft" => "“",
        "quotedblright" => "”",
        "guillemotleft" => "«",
        "guillemotright" => "»",
        "parenleft" => "(",
        "parenright" => ")",
        "asterisk" => "*",
        "plus" => "+",
        "comma" => ",",
        "hyphen" | "minus" | "sfthyphen" => "-",
        "endash" => "–",
        "emdash" => "—",
        "period" => ".",
        "slash" => "/",
        "zero" => "0",
        "one" => "1",
        "two" => "2",
        "three" => "3",
        "four" => "4",
        "five" => "5",
        "six" => "6",
        "seven" => "7",
        "eight" => "8",
        "nine" => "9",
        "colon" => ":",
        "semicolon" => ";",
        "less" => "<",
        "equal" => "=",
        "greater" => ">",
        "question" => "?",
        "at" => "@",
        "Euro" | "euro" => "€",
        "eacute" => "é",
        "egrave" => "è",
        "ecircumflex" => "ê",
        "edieresis" => "ë",
        "aacute" => "á",
        "agrave" => "à",
        "acircumflex" => "â",
        "ccedilla" => "ç",
        "ugrave" => "ù",
        "ucircumflex" => "û",
        "ocircumflex" => "ô",
        "icircumflex" => "î",
        "idieresis" => "ï",
        "oe" => "œ",
        "OE" => "Œ",
        "ae" => "æ",
        "AE" => "Æ",
        "Eacute" => "É",
        "Egrave" => "È",
        "Ecircumflex" => "Ê",
        "Agrave" => "À",
        "Ccedilla" => "Ç",
        "fi" | "f_i" => "fi",
        "fl" | "f_l" => "fl",
        "ff" | "f_f" => "ff",
        "ffi" | "f_f_i" => "ffi",
        "ffl" | "f_f_l" => "ffl",
        "bullet" => "•",
        "degree" => "°",
        "numero" => "№",
        _ if base.len() == 1 && base.as_bytes()[0].is_ascii_alphabetic() => base,
        _ => return None,
    };
    Some(ch.to_string())
}

fn build_font_ir_from_name(
    raw_name: String,
    fd_weight: Option<u16>,
    fd_italic: bool,
    fd_mono: bool,
    fd_serif: bool,
) -> FontIr {
    let is_subset = raw_name.split_once('+').is_some_and(|(p, _)| p.len() == 6);
    let clean_name = match raw_name.split_once('+') {
        Some((p, rest)) if p.len() == 6 => rest.to_string(),
        _ => raw_name.clone(),
    };
    let lower = clean_name.to_ascii_lowercase();
    let is_bold = fd_weight.is_some_and(|w| w >= 600)
        || lower.contains("bold")
        || lower.contains("gras")
        || lower.contains("black")
        || lower.contains("demi")
        || lower.contains("semibold")
        || lower.contains("heavy");
    let is_italic =
        fd_italic || lower.contains("italic") || lower.contains("italique") || lower.contains("oblique");
    let is_monospace =
        fd_mono || lower.contains("mono") || lower.contains("courier") || lower.contains("consolas");
    let is_serif = fd_serif
        || lower.contains("times")
        || lower.contains("georgia")
        || lower.contains("perpetua")
        || lower.contains("garamond")
        || lower.contains("baskerville")
        || lower.contains("palatino")
        || lower.contains("cambria")
        || lower.contains("minion")
        || lower.contains("bodoni")
        || lower.contains("caslon")
        || (lower.contains("serif") && !lower.contains("sans"));
    FontIr {
        postscript_name: raw_name,
        family_name: clean_name,
        is_bold,
        is_italic,
        is_monospace,
        is_serif,
        is_embedded_subset: is_subset,
        size_scale: None,
        program: None,
        glyph_widths: BTreeMap::new(),
        shapes: BTreeMap::new(),
    }
}

/// The character `s` stands for, when it is exactly one.
fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    chars.next().filter(|_| chars.next().is_none())
}


/// `path` (visual page points) in the em space of a glyph drawn at `origin`, `size` points per em
/// and stretched by `stretch`.
fn em_shape(path: &BezPath, origin: Point, size: f32, stretch: f32) -> Vec<PathOp> {
    let sx = 1000.0 / (size * stretch.max(0.05)) as f64;
    let sy = 1000.0 / size as f64;
    let p = |pt: Point| [((pt.x - origin.x) * sx) as f32, ((origin.y - pt.y) * sy) as f32];
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(a) => PathOp::MoveTo(p(a)),
            PathEl::LineTo(a) => PathOp::LineTo(p(a)),
            PathEl::QuadTo(a, b) => {
                let (a, b) = (p(a), p(b));
                PathOp::QuadTo([a[0], a[1], b[0], b[1]])
            }
            PathEl::CurveTo(a, b, c) => {
                let (a, b, c) = (p(a), p(b), p(c));
                PathOp::CurveTo([a[0], a[1], b[0], b[1], c[0], c[1]])
            }
            PathEl::ClosePath => PathOp::Close,
        })
        .collect()
}

/// Collects what a Type 3 glyph procedure fills, to keep the glyph's shape.
#[derive(Default)]
struct ShapeCapture {
    path: BezPath,
    /// The glyph draws something that is not a filled outline (an image, a stroke).
    other: bool,
}

impl<'a> Device<'a> for ShapeCapture {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        match draw_mode {
            DrawMode::Fill(_) => self.path.extend(props.transform * path.clone()),
            DrawMode::Invisible => {}
            _ => self.other = true,
        }
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {
        self.other = true;
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {
        self.other = true;
    }
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// A Type 3 font of a page, so its glyphs (which hayro does not tie to a font) can be told apart
/// and get their real advances.
struct Type3Font {
    name: String,
    first_char: u32,
    /// `/Widths`, in glyph space.
    widths: Vec<f32>,
    /// `FontMatrix` scale: glyph space to text space, horizontally and vertically.
    scale: (f32, f32),
    /// Character each code stands for (`ToUnicode`).
    chars: std::collections::HashMap<u32, char>,
    /// Its entry in the page's fonts, once a glyph used it.
    font_idx: Option<usize>,
}

impl Type3Font {
    fn width(&self, code: u32) -> Option<f32> {
        let w = *self.widths.get(code.checked_sub(self.first_char)? as usize)?;
        (w > 0.0).then_some(w)
    }
}

/// The Type 3 fonts a page (and the forms it draws) can use.
fn page_type3_fonts(resources: &hayro_interpret::hayro_syntax::page::Resources<'_>) -> Vec<Type3Font> {
    fn collect(fonts: &Dict<'_>, x_objects: &Dict<'_>, depth: usize, out: &mut Vec<Type3Font>) {
        for key in fonts.keys() {
            let Some(font) = fonts.get::<Dict<'_>>(key.as_ref()) else {
                continue;
            };
            if font.get::<Name<'_>>(SUBTYPE).is_none_or(|s| s.as_ref() != b"Type3") {
                continue;
            }
            let matrix: Vec<f32> = font
                .get::<Array<'_>>(FONT_MATRIX)
                .map(|m| m.iter::<f32>().collect())
                .unwrap_or_default();
            let name = font
                .get::<Dict<'_>>(FONT_DESC)
                .and_then(|d| d.get::<Name<'_>>(FONT_NAME))
                .or_else(|| font.get::<Name<'_>>(BASE_FONT))
                .or_else(|| font.get::<Name<'_>>(NAME))
                .map(|n| String::from_utf8_lossy(n.as_ref()).to_string())
                .unwrap_or_default();
            let t3 = Type3Font {
                name,
                first_char: font.get::<i32>(FIRST_CHAR).unwrap_or(0).max(0) as u32,
                widths: font.get::<Array<'_>>(WIDTHS).map(|w| w.iter::<f32>().collect()).unwrap_or_default(),
                scale: (
                    matrix.first().map_or(0.001, |v| v.abs()),
                    matrix.get(3).map_or(0.001, |v| v.abs()),
                ),
                chars: tounicode::to_unicode_chars(&font),
                font_idx: None,
            };
            // The same font object reached twice (page and form) counts once.
            if !out.iter().any(|o| o.name == t3.name && o.widths == t3.widths && o.first_char == t3.first_char) {
                out.push(t3);
            }
        }
        if depth >= 3 {
            return;
        }
        for key in x_objects.keys() {
            let Some(form) = x_objects.get::<hayro_interpret::hayro_syntax::object::Stream<'_>>(key.as_ref()) else {
                continue;
            };
            if let Some(res) = form.dict().get::<Dict<'_>>(RESOURCES) {
                let empty = Dict::default();
                let fonts = res.get::<Dict<'_>>(FONT).unwrap_or_else(|| empty.clone());
                let x_objects = res.get::<Dict<'_>>(XOBJECT).unwrap_or(empty);
                collect(&fonts, &x_objects, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    collect(&resources.fonts, &resources.x_objects, 0, &mut out);
    out
}

/// Codes and widths seen in a font, and the PDF width table its name was taken from.
type WidthNaming = (Vec<(u32, i32)>, Option<usize>);

struct HayroExtractDevice {
    _page_index: usize,
    page_rect: RectPt,
    extract_images: bool,
    default_tc: f32,
    default_tw: f32,
    clip_stack: Vec<RectPt>,
    opacity_stack: Vec<f32>,
    mc_stack: Vec<(Option<i32>, Option<String>)>,
    fonts: Vec<FontIr>,
    font_key_to_idx: BTreeMap<u128, usize>,
    /// For fonts named from the PDF's width tables (no program to read a name from): the codes and
    /// widths seen so far, and the table the name came from.
    named_by_widths: BTreeMap<usize, WidthNaming>,
    font_cmap_cache: BTreeMap<u128, InvertedFontMap>,
    font_scale_cache: BTreeMap<u128, f32>,
    empty_glyph_cache: BTreeMap<(u128, u32), bool>,
    /// Embedded font programs by font, shared with the other pages.
    programs: BTreeMap<u128, FontProgram>,
    pdf_font_tables: Arc<[(String, u32, Vec<i32>)]>,
    type3_fonts: Vec<Type3Font>,
    glyphs: Vec<GlyphIr>,
    paths: Vec<PathIr>,
    images: Vec<ImageIr>,
    cmap_fallback_recoveries: usize,
    /// Something was drawn that plain rectangles and rules cannot reproduce.
    complex_graphics: bool,
}

impl HayroExtractDevice {
    fn new(
        page_index: usize,
        width_pt: f32,
        height_pt: f32,
        extract_images: bool,
        default_tc: f32,
        default_tw: f32,
        pdf_font_tables: Arc<[(String, u32, Vec<i32>)]>,
    ) -> Self {
        let page_rect = RectPt::new(0.0, 0.0, width_pt, height_pt);
        Self {
            _page_index: page_index,
            page_rect,
            extract_images,
            default_tc,
            default_tw,
            clip_stack: vec![page_rect],
            opacity_stack: Vec::new(),
            mc_stack: Vec::new(),
            fonts: vec![FontIr {
                postscript_name: "Helvetica".to_string(),
                family_name: "Helvetica".to_string(),
                is_bold: false,
                is_italic: false,
                is_monospace: false,
                is_serif: false,
                is_embedded_subset: false,
                size_scale: None,
                program: None,
                glyph_widths: BTreeMap::new(),
                shapes: BTreeMap::new(),
            }],
            font_key_to_idx: BTreeMap::new(),
            named_by_widths: BTreeMap::new(),
            font_cmap_cache: BTreeMap::new(),
            font_scale_cache: BTreeMap::new(),
            empty_glyph_cache: BTreeMap::new(),
            programs: BTreeMap::new(),
            pdf_font_tables,
            type3_fonts: Vec::new(),
            glyphs: Vec::new(),
            paths: Vec::new(),
            images: Vec::new(),
            cmap_fallback_recoveries: 0,
            complex_graphics: false,
        }
    }

    fn is_visible_in_active_clip(&self, bbox: &RectPt) -> bool {
        if self.clip_stack.len() <= 1 {
            // No explicit W/W* clipping path pushed: only reject wildly off-page coordinates
            let expanded = RectPt::new(
                -self.page_rect.width(),
                -self.page_rect.height(),
                self.page_rect.x1 * 2.5,
                self.page_rect.y1 * 2.5,
            );
            expanded.intersection(bbox).is_some()
        } else {
            self.clip_stack
                .last()
                .copied()
                .unwrap_or(self.page_rect)
                .intersection(bbox)
                .is_some()
        }
    }

    fn current_mcid(&self) -> Option<i32> {
        self.mc_stack.iter().rev().find_map(|(mcid, _)| *mcid)
    }

    fn current_actual_text(&self) -> Option<String> {
        self.mc_stack
            .iter()
            .rev()
            .find_map(|(_, actual)| actual.clone())
    }

    fn paint_to_rgba(&self, paint: &Paint<'_>) -> RgbaColor {
        let group_alpha = self.opacity_stack.iter().copied().product::<f32>().clamp(0.0, 1.0);
        match paint {
            Paint::Color(c) => {
                let [r, g, b, a] = c.to_rgba().to_rgba8();
                let a = (a as f32 * group_alpha).round().clamp(0.0, 255.0) as u8;
                RgbaColor { r, g, b, a }
            }
            Paint::Pattern(_) => RgbaColor {
                r: 0,
                g: 0,
                b: 0,
                a: (255.0 * group_alpha).round().clamp(0.0, 255.0) as u8,
            },
        }
    }

    /// Whether the PDF width table `t` agrees with every `(code, width)` of `seen`.
    fn width_table_matches(&self, t: usize, seen: &[(u32, i32)]) -> bool {
        let (_, fc, widths) = &self.pdf_font_tables[t];
        seen.iter().all(|&(code, adv)| {
            code.checked_sub(*fc)
                .and_then(|i| widths.get(i as usize))
                .is_some_and(|w| (*w - adv).abs() <= 1)
        })
    }

    /// The first PDF width table that agrees with every `(code, width)` of `seen`.
    fn width_table_for(&self, seen: &[(u32, i32)]) -> Option<usize> {
        (0..self.pdf_font_tables.len()).find(|&t| self.width_table_matches(t, seen))
    }

    /// Fonts without a readable program are named after the PDF font whose widths match their
    /// glyphs. One glyph can match several fonts, so the name is checked against every glyph and
    /// corrected when a later one rules it out.
    fn check_width_name(&mut self, font_idx: usize, glyph: &hayro_interpret::font::OutlineGlyph) {
        let Some((seen, table)) = self.named_by_widths.get_mut(&font_idx) else {
            return;
        };
        let pair = (glyph.char_code(), glyph.advance_width().unwrap_or(0.0).round() as i32);
        if seen.contains(&pair) {
            return;
        }
        seen.push(pair);
        let (seen, table) = (seen.clone(), *table);
        if table.is_some_and(|t| self.width_table_matches(t, &[pair])) {
            return;
        }
        let Some(found) = self.width_table_for(&seen) else {
            return;
        };
        if Some(found) != table {
            let name = self.pdf_font_tables[found].0.clone();
            let renamed = build_font_ir_from_name(name, None, false, false, false);
            let font = &mut self.fonts[font_idx];
            font.postscript_name = renamed.postscript_name;
            font.family_name = renamed.family_name;
            font.is_bold = renamed.is_bold;
            font.is_italic = renamed.is_italic;
            font.is_monospace = renamed.is_monospace;
            font.is_serif = renamed.is_serif;
            font.is_embedded_subset = renamed.is_embedded_subset;
            if let Some(entry) = self.named_by_widths.get_mut(&font_idx) {
                entry.1 = Some(found);
            }
        }
    }

    /// Which of the page's Type 3 fonts drew `glyph_run` (hayro does not tie Type 3 glyphs to their
    /// font). A run is drawn in one font: the one that has every code of the run, for the character
    /// it stands for when `ToUnicode` says so, and whose advances fit the glyph positions best
    /// when several do. `None` when no font has them.
    fn run_type3_font(&self, glyph_run: &GlyphRun<'_, '_>, transform: Affine) -> Option<usize> {
        let glyphs = glyph_run.glyphs();
        let mut candidates: Option<Vec<usize>> = None;
        // Each code, the distance to the next glyph on the line and the text space em.
        let mut seen: Vec<(u32, Option<f32>, f32)> = Vec::new();
        for (k, g) in glyphs.iter().enumerate() {
            let Glyph::Type3(t3) = &**g else {
                continue;
            };
            let code = t3.char_code();
            let single = t3.as_unicode().and_then(|bf| match bf {
                BfString::Char(c) => Some(c),
                BfString::String(s) => single_char(&s),
            });
            let with_code: Vec<usize> =
                (0..self.type3_fonts.len()).filter(|&i| self.type3_fonts[i].width(code).is_some()).collect();
            let named: Vec<usize> = with_code
                .iter()
                .copied()
                .filter(|&i| single.is_some() && self.type3_fonts[i].chars.get(&code).copied() == single)
                .collect();
            let fits = if named.is_empty() { with_code } else { named };
            // A code no font gives an advance (a zero-width mark, a code outside `/Widths`) does
            // not tell the fonts apart.
            if !fits.is_empty() {
                candidates = Some(match candidates {
                    None => fits,
                    Some(c) => c.into_iter().filter(|i| fits.contains(i)).collect(),
                });
            }
            let tf = transform * g.transform();
            let origin = tf * Point::ZERO;
            let em = tf * Point::new(1000.0, 0.0);
            let dx = glyphs.get(k + 1).and_then(|next| {
                let o = (transform * next.transform()) * Point::ZERO;
                ((o.y - origin.y).abs() <= 1.5).then_some((o.x - origin.x) as f32)
            });
            seen.push((code, dx, libm::hypotf((em.x - origin.x) as f32, (em.y - origin.y) as f32)));
        }
        let candidates = candidates?;
        let advance = |i: usize, code: u32, em: f32| {
            let f = &self.type3_fonts[i];
            f.width(code).unwrap_or(0.0) * f.scale.0 * em
        };
        match candidates.as_slice() {
            [] => None,
            [one] => Some(*one),
            [first, rest @ ..] => {
                // Fonts with the same advances draw the run alike; otherwise keep the one whose
                // advances come closest to the glyph positions (letter spacing and `TJ`
                // adjustments shift every candidate alike; the first one on a tie).
                let alike = rest
                    .iter()
                    .all(|&i| seen.iter().all(|&(code, _, em)| (advance(i, code, em) - advance(*first, code, em)).abs() < 0.01));
                if alike {
                    return Some(*first);
                }
                let error = |i: usize| -> f32 {
                    seen.iter()
                        .filter_map(|&(code, dx, em)| dx.map(|dx| (advance(i, code, em) - dx).abs()))
                        .sum()
                };
                candidates.iter().copied().min_by(|&a, &b| error(a).total_cmp(&error(b)))
            }
        }
    }

    fn intern_font(&mut self, glyph: &hayro_interpret::font::OutlineGlyph) -> usize {
        let key = glyph.font_cache_key();
        if let Some(&idx) = self.font_key_to_idx.get(&key) {
            self.check_width_name(idx, glyph);
            return idx;
        }
        let font_ir = if let Some(fd) = glyph.font_data() {
            let raw_name = fd
                .postscript_name
                .clone()
                .unwrap_or_else(|| "Helvetica".to_string());
            let mut ir =
                build_font_ir_from_name(raw_name, fd.weight.map(|w| w as u16), fd.is_italic, fd.is_monospace, fd.is_serif);
            let program = self.programs.entry(key).or_insert_with(|| FontProgram(Arc::from((*fd.data).as_ref())));
            ir.program = Some(program.clone());
            ir
        } else {
            let pair = (glyph.char_code(), glyph.advance_width().unwrap_or(0.0).round() as i32);
            let table = self.width_table_for(&[pair]);
            self.named_by_widths.insert(self.fonts.len(), (vec![pair], table));
            if let Some(t) = table {
                build_font_ir_from_name(self.pdf_font_tables[t].0.clone(), None, false, false, false)
            } else {
                let dbg = format!("{glyph:?}");
                let is_serif = dbg.contains("Times");
                let is_monospace = dbg.contains("Courier");
                let is_bold = dbg.contains("Bold");
                let is_italic = dbg.contains("Italic") || dbg.contains("Oblique");
                let family_name = if is_serif {
                    "Times New Roman".to_string()
                } else if is_monospace {
                    "Courier".to_string()
                } else {
                    "Helvetica".to_string()
                };
                let postscript_name = match (is_serif, is_monospace, is_bold, is_italic) {
                    (true, _, true, true) => "Times-BoldItalic",
                    (true, _, true, false) => "Times-Bold",
                    (true, _, false, true) => "Times-Italic",
                    (true, _, false, false) => "Times-Roman",
                    (_, true, true, true) => "Courier-BoldOblique",
                    (_, true, true, false) => "Courier-Bold",
                    (_, true, false, true) => "Courier-Oblique",
                    (_, true, false, false) => "Courier",
                    (_, _, true, true) => "Helvetica-BoldOblique",
                    (_, _, true, false) => "Helvetica-Bold",
                    (_, _, false, true) => "Helvetica-Oblique",
                    _ => "Helvetica",
                }
                .to_string();
                FontIr {
                    postscript_name,
                    family_name,
                    is_bold,
                    is_italic,
                    is_monospace,
                    is_serif,
                    is_embedded_subset: false,
                    size_scale: None,
                    program: None,
                    glyph_widths: BTreeMap::new(),
                    shapes: BTreeMap::new(),
                }
            }
        };
        let idx = self.fonts.len();
        self.fonts.push(font_ir);
        self.font_key_to_idx.insert(key, idx);
        idx
    }

    fn resolve_outline_unicode(
        &mut self,
        glyph: &hayro_interpret::font::OutlineGlyph,
    ) -> Option<String> {
        let font_key = glyph.font_cache_key();
        let gid_u32 = glyph.glyph_id().to_u32();
        // An outline glyph with zero contour elements draws no ink on the page and is a whitespace glyph
        let is_empty = *self
            .empty_glyph_cache
            .entry((font_key, gid_u32))
            .or_insert_with(|| glyph.outline().elements().is_empty());
        if is_empty {
            return Some(" ".to_string());
        }

        if let Some(bf) = glyph.as_unicode() {
            let s = match bf {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            };
            if !s.is_empty() && !s.contains('\u{FFFD}') {
                return Some(s);
            }
        }

        // Custom `skrifa` embedded font `cmap`/`post` table inversion fallback
        if !self.font_cmap_cache.contains_key(&font_key) {
            if let Some(fd) = glyph.font_data() {
                let bytes: &[u8] = (*fd.data).as_ref();
                let map = InvertedFontMap::from_font_bytes(bytes);
                self.font_cmap_cache.insert(font_key, map);
            }
        }

        let gid = gid_u32 as u16;
        if let Some(inv) = self.font_cmap_cache.get(&font_key) {
            if let Some(u) = inv.gid_to_unicode.get(&gid) {
                self.cmap_fallback_recoveries += 1;
                return Some(u.clone());
            }
        }

        // Last-resort ASCII char_code fallback if printable ASCII
        let code = glyph.char_code();
        if (0x20..=0x7E).contains(&code) {
            return char::from_u32(code).map(|c| c.to_string());
        }

        None
    }
}

impl<'a> Device<'a> for HayroExtractDevice {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        let transformed = props.transform * path.clone();
        let bb = transformed.bounding_box();
        let bbox = RectPt::new(bb.x0 as f32, bb.y0 as f32, bb.x1 as f32, bb.y1 as f32);
        if bbox.width() <= 0.01 && bbox.height() <= 0.01 {
            return;
        }

        if matches!(props.paint, Paint::Pattern(_)) || props.soft_mask.is_some() || props.blend_mode != BlendMode::Normal {
            self.complex_graphics = true;
        }
        let color = self.paint_to_rgba(&props.paint);
        let (stroke_color, fill_color, stroke_width_pt) = match draw_mode {
            DrawMode::Fill(_) => (None, Some(color), 0.0),
            DrawMode::Stroke(sp) => {
                let scale = extract_scale_factor(props.transform);
                (Some(color), None, (sp.line_width * scale).max(0.25))
            }
            DrawMode::FillAndStroke(_, sp) => {
                let scale = extract_scale_factor(props.transform);
                (Some(color), Some(color), (sp.line_width * scale).max(0.25))
            }
            DrawMode::Invisible => return,
        };

        let elements = transformed.elements();
        let segment_count = elements.len();
        let is_horizontal_line = bbox.height() <= 2.5 && bbox.width() >= 8.0;
        let is_vertical_line = bbox.width() <= 2.5 && bbox.height() >= 8.0;
        let is_axis_aligned_rect = is_axis_aligned_box(elements);
        let clip = self.clip_stack.last().copied().unwrap_or(self.page_rect);
        let clipped = bbox.x0 < clip.x0 - 0.5 || bbox.y0 < clip.y0 - 0.5 || bbox.x1 > clip.x1 + 0.5 || bbox.y1 > clip.y1 + 0.5;
        if clipped && self.clip_stack.len() > 1 {
            self.complex_graphics = true;
        }

        // If a stroked compound path contains multiple horizontal/vertical grid lines (e.g. Excel/Sheets tables),
        // decompose its axis-aligned segments into individual line PathIrs so lattice table detection sees every line!
        if stroke_color.is_some()
            && fill_color.is_none()
            && !is_horizontal_line
            && !is_vertical_line
            && !is_axis_aligned_rect
            && segment_count >= 4
        {
            let mut cur_pt: Option<Point> = None;
            let mut start_pt: Option<Point> = None;
            let mut emitted_segments = 0usize;
            for el in elements {
                match *el {
                    PathEl::MoveTo(p) => {
                        cur_pt = Some(p);
                        start_pt = Some(p);
                    }
                    PathEl::LineTo(p) => {
                        if let Some(prev) = cur_pt {
                            let seg_bb = RectPt::from_points(
                                prev.x as f32,
                                prev.y as f32,
                                p.x as f32,
                                p.y as f32,
                            );
                            let h_line = seg_bb.height() <= 2.5 && seg_bb.width() >= 8.0;
                            let v_line = seg_bb.width() <= 2.5 && seg_bb.height() >= 8.0;
                            if h_line || v_line {
                                self.paths.push(PathIr {
                                    bbox: seg_bb,
                                    stroke_color,
                                    fill_color: None,
                                    stroke_width_pt,
                                    is_axis_aligned_rect: false,
                                    is_horizontal_line: h_line,
                                    is_vertical_line: v_line,
                                    segment_count: 2,
                                });
                                emitted_segments += 1;
                            }
                        }
                        cur_pt = Some(p);
                    }
                    PathEl::ClosePath => {
                        if let (Some(prev), Some(st)) = (cur_pt, start_pt) {
                            let seg_bb = RectPt::from_points(
                                prev.x as f32,
                                prev.y as f32,
                                st.x as f32,
                                st.y as f32,
                            );
                            let h_line = seg_bb.height() <= 2.5 && seg_bb.width() >= 8.0;
                            let v_line = seg_bb.width() <= 2.5 && seg_bb.height() >= 8.0;
                            if h_line || v_line {
                                self.paths.push(PathIr {
                                    bbox: seg_bb,
                                    stroke_color,
                                    fill_color: None,
                                    stroke_width_pt,
                                    is_axis_aligned_rect: false,
                                    is_horizontal_line: h_line,
                                    is_vertical_line: v_line,
                                    segment_count: 2,
                                });
                                emitted_segments += 1;
                            }
                        }
                        cur_pt = start_pt;
                    }
                    _ => {
                        cur_pt = None;
                    }
                }
            }
            if emitted_segments >= 2 {
                return;
            }
        }
        if !(is_axis_aligned_rect || is_horizontal_line || is_vertical_line) {
            self.complex_graphics = true;
        }

        self.paths.push(PathIr {
            bbox,
            stroke_color,
            fill_color,
            stroke_width_pt,
            is_axis_aligned_rect,
            is_horizontal_line,
            is_vertical_line,
            segment_count,
        });
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        let bb = clip_path.path.bounding_box();
        let rect = RectPt::new(bb.x0 as f32, bb.y0 as f32, bb.x1 as f32, bb.y1 as f32);
        let mut clip_base = self
            .clip_stack
            .last()
            .copied()
            .unwrap_or(self.page_rect);
        if self.clip_stack.len() == 1 {
            clip_base = self.page_rect;
        }
        let intersected = clip_base
            .intersection(&rect)
            .unwrap_or(RectPt::new(0.0, 0.0, 0.0, 0.0));
        self.clip_stack.push(intersected);
    }

    fn pop_clip(&mut self) {
        if self.clip_stack.len() > 1 {
            self.clip_stack.pop();
        }
    }

    fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'a>>,
        blend_mode: BlendMode,
    ) {
        self.opacity_stack.push(opacity.clamp(0.0, 1.0));
        if opacity < 0.999 || mask.is_some() || blend_mode != BlendMode::Normal {
            self.complex_graphics = true;
        }
    }

    fn pop_transparency_group(&mut self) {
        self.opacity_stack.pop();
    }

    fn begin_marked_content(&mut self, _tag: &[u8], properties: &MarkedContentProperties<'_>) {
        let mcid = properties.mcid();
        let actual_text = properties.actual_text().map(|s| decode_pdf_string(&s));
        self.mc_stack.push((mcid, actual_text));
    }

    fn end_marked_content(&mut self) {
        self.mc_stack.pop();
    }

    fn draw_glyph_run(
        &mut self,
        glyph_run: &GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        let render_mode = match draw_mode {
            DrawMode::Fill(_) => RenderMode::Fill,
            DrawMode::Stroke(_) => RenderMode::Stroke,
            DrawMode::FillAndStroke(_, _) => RenderMode::FillAndStroke,
            DrawMode::Invisible => RenderMode::Invisible,
        };
        let color = self.paint_to_rgba(&props.paint);
        let mcid = self.current_mcid();
        let actual_text = self.current_actual_text();
        let run_type3 = self.run_type3_font(glyph_run, props.transform);

        for (idx_in_run, pos_glyph) in glyph_run.glyphs().iter().enumerate() {
            // If parent `/Span` has `ActualText`, attach it to the first glyph of the span
            let span_actual = if idx_in_run == 0 {
                actual_text.clone()
            } else {
                None
            };

            let full_tf = props.transform * pos_glyph.transform();
            let origin = full_tf * Point::new(0.0, 0.0);
            let em_x = full_tf * Point::new(1000.0, 0.0);
            let em_y = full_tf * Point::new(0.0, -1000.0);

            let dx_em = (em_x.x - origin.x) as f32;
            let dy_em = (em_x.y - origin.y) as f32;
            let width_em = libm::hypotf(dx_em, dy_em);
            let height_em = libm::hypotf((em_y.x - origin.x) as f32, (em_y.y - origin.y) as f32);

            let font_size_pt = height_em.max(width_em).max(1.0);
            let horiz_scale = if height_em > 0.1 {
                (width_em / height_em).clamp(0.25, 4.0)
            } else {
                1.0
            };
            let angle_deg = libm::atan2f(dy_em, dx_em).to_degrees();

            match &**pos_glyph {
                Glyph::Outline(outline_glyph) => {
                    let font_key = outline_glyph.font_cache_key();
                    let font_idx = self.intern_font(outline_glyph);
                    let unicode = self
                        .resolve_outline_unicode(outline_glyph)
                        .or_else(|| span_actual.clone())
                        .unwrap_or_else(|| "\u{FFFD}".to_string());

                    if !self.font_scale_cache.contains_key(&font_key)
                        && unicode.len() == 1
                        && matches!(
                            unicode.as_bytes()[0],
                            b'A'..=b'I' | b'K'..=b'P' | b'R'..=b'Z' | b'0'..=b'9'
                        )
                        && !self.fonts[font_idx].is_serif
                        && !self.fonts[font_idx].is_monospace
                        && font_size_pt > 2.0
                    {
                        let ink_bb = (full_tf * outline_glyph.outline().clone()).bounding_box();
                        let ink_h = ink_bb.height() as f32;
                        if ink_h > 1.0 {
                            let ratio = ink_h / font_size_pt;
                            let scale = if (0.35..=0.57).contains(&ratio) {
                                (ratio / 0.70).clamp(0.68, 1.0)
                            } else {
                                1.0
                            };
                            self.font_scale_cache.insert(font_key, scale);
                            if (scale - 1.0).abs() > 0.01 {
                                self.fonts[font_idx].size_scale = Some(scale);
                                // Shapes kept so far were measured against the unscaled size.
                                for shape in self.fonts[font_idx].shapes.values_mut() {
                                    shape.advance /= scale;
                                    for op in &mut shape.path {
                                        let points: &mut [f32] = match op {
                                            PathOp::MoveTo(p) | PathOp::LineTo(p) => p,
                                            PathOp::QuadTo(p) => p,
                                            PathOp::CurveTo(p) => p,
                                            PathOp::Close => &mut [],
                                        };
                                        points.iter_mut().for_each(|v| *v /= scale);
                                    }
                                }
                                for g in &mut self.glyphs {
                                    if g.font_idx == font_idx {
                                        g.font_size_pt *= scale;
                                        g.bbox.y0 = g.baseline_y - g.font_size_pt * 0.82;
                                        g.bbox.y1 = g.baseline_y + g.font_size_pt * 0.22;
                                    }
                                }
                            }
                        }
                    }

                    let font_scale = self.font_scale_cache.get(&font_key).copied().unwrap_or(1.0);
                    let font_size_pt = (font_size_pt * font_scale).max(1.0);

                    let adv_units = outline_glyph.advance_width().unwrap_or(500.0);
                    if let Some(w) = outline_glyph.advance_width() {
                        let gid = outline_glyph.glyph_id().to_u32() as u16;
                        self.fonts[font_idx].glyph_widths.entry(gid).or_insert(w);
                    }
                    let adv_pt_end = full_tf * Point::new(adv_units as f64, 0.0);
                    let advance_pt = libm::hypotf(
                        (adv_pt_end.x - origin.x) as f32,
                        (adv_pt_end.y - origin.y) as f32,
                    )
                    .max(font_size_pt * 0.2);

                    // Keep each glyph's shape: the editor draws fonts a browser cannot load (Type 1,
                    // CFF) from them, and missing fonts are matched to installed ones by them.
                    let gid = outline_glyph.glyph_id().to_u32() as u16;
                    if angle_deg.abs() < 0.5
                        && !self.fonts[font_idx].shapes.contains_key(&gid)
                        && let Some(c) = single_char(&unicode)
                    {
                        let path = full_tf * outline_glyph.outline();
                        self.fonts[font_idx].shapes.insert(
                            gid,
                            GlyphShape {
                                unicode: c,
                                advance: advance_pt / (font_size_pt * horiz_scale) * 1000.0,
                                path: em_shape(&path, origin, font_size_pt, horiz_scale),
                            },
                        );
                    }

                    let top_y = (origin.y as f32) - font_size_pt * 0.82;
                    let bot_y = (origin.y as f32) + font_size_pt * 0.22;
                    let bbox = RectPt::from_points(
                        origin.x as f32,
                        top_y,
                        (origin.x as f32) + advance_pt,
                        bot_y,
                    );

                    let is_visible_in_clip = self.is_visible_in_active_clip(&bbox);

                    self.glyphs.push(GlyphIr {
                        unicode,
                        glyph_id: outline_glyph.glyph_id().to_u32() as u16,
                        bbox,
                        baseline_y: origin.y as f32,
                        font_size_pt,
                        advance_pt,
                        char_spacing_pt: self.default_tc,
                        word_spacing_pt: self.default_tw,
                        horiz_scale,
                        angle_deg,
                        font_idx,
                        color,
                        render_mode,
                        mcid,
                        actual_text: span_actual,
                        is_visible_in_clip,
                    });
                }
                Glyph::Type3(t3_glyph) => {
                    let unicode = t3_glyph
                        .as_unicode()
                        .map(|bf| match bf {
                            BfString::Char(c) => c.to_string(),
                            BfString::String(s) => s,
                        })
                        .or_else(|| span_actual.clone())
                        .unwrap_or_else(|| "\u{FFFD}".to_string());

                    let code = t3_glyph.char_code();
                    let single = single_char(&unicode);
                    // A glyph without an advance in the run's font keeps the old fallback.
                    let t3 = run_type3.filter(|&i| self.type3_fonts[i].width(code).is_some());
                    let (font_idx, font_size_pt, advance) = match t3 {
                        Some(i) => {
                            let f = &self.type3_fonts[i];
                            let width = f.width(code).unwrap_or(0.0);
                            // `width_em`/`height_em` are the text space em (the size set by `Tf`);
                            // the font matrix takes glyph space widths to it.
                            let size = height_em.max(1.0);
                            let advance = width * f.scale.0 * width_em;
                            let w1000 = width * f.scale.0 * 1000.0;
                            let font_idx = match f.font_idx {
                                Some(idx) => idx,
                                None => {
                                    let ir = build_font_ir_from_name(f.name.clone(), None, false, false, false);
                                    self.fonts.push(ir);
                                    let idx = self.fonts.len() - 1;
                                    self.type3_fonts[i].font_idx = Some(idx);
                                    idx
                                }
                            };
                            self.fonts[font_idx].glyph_widths.entry(code as u16).or_insert(w1000);
                            (font_idx, size, Some(advance))
                        }
                        None => (0, font_size_pt, None),
                    };

                    let char_em = match unicode.chars().next() {
                        Some('m' | 'w' | 'M' | 'W' | 'œ' | 'Œ' | 'æ' | 'Æ') => 0.88,
                        Some('i' | 'l' | 'I' | 'j' | '.' | ',' | ':' | ';' | '!' | '\'' | '’' | '‘') => 0.30,
                        Some(' ' | '\u{00A0}' | '\u{202F}') => 0.33,
                        Some(c) if c.is_uppercase() => 0.68,
                        _ => 0.56,
                    };
                    let mut advance_pt = font_size_pt * char_em;
                    if let Some(next_g) = glyph_run.glyphs().get(idx_in_run + 1) {
                        let next_origin = (props.transform * next_g.transform()) * Point::new(0.0, 0.0);
                        let dx = (next_origin.x - origin.x) as f32;
                        let dy = (next_origin.y - origin.y) as f32;
                        if dy.abs() <= 1.5 && dx >= font_size_pt * 0.15 && dx <= font_size_pt * 1.15 {
                            advance_pt = dx;
                        }
                    }
                    let advance_pt = advance.unwrap_or(advance_pt);
                    if t3.is_some()
                        && angle_deg.abs() < 0.5
                        && !self.fonts[font_idx].shapes.contains_key(&(code as u16))
                        && let Some(c) = single
                    {
                        let mut capture = ShapeCapture::default();
                        t3_glyph.interpret(&mut capture, props.transform, pos_glyph.transform(), &props.paint);
                        if !capture.other {
                            self.fonts[font_idx].shapes.insert(
                                code as u16,
                                GlyphShape {
                                    unicode: c,
                                    advance: advance_pt / (font_size_pt * horiz_scale) * 1000.0,
                                    path: em_shape(&capture.path, origin, font_size_pt, horiz_scale),
                                },
                            );
                        }
                    }

                    let bbox = RectPt::from_points(
                        origin.x as f32,
                        (origin.y as f32) - font_size_pt * 0.8,
                        (origin.x as f32) + advance_pt,
                        (origin.y as f32) + font_size_pt * 0.2,
                    );
                    let is_visible_in_clip = self.is_visible_in_active_clip(&bbox);

                    self.glyphs.push(GlyphIr {
                        unicode,
                        glyph_id: code as u16,
                        bbox,
                        baseline_y: origin.y as f32,
                        font_size_pt,
                        advance_pt,
                        char_spacing_pt: self.default_tc,
                        word_spacing_pt: self.default_tw,
                        horiz_scale,
                        angle_deg,
                        font_idx,
                        color,
                        render_mode,
                        mcid,
                        actual_text: span_actual,
                        is_visible_in_clip,
                    });
                }
            }
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        self.complex_graphics = true;
        match image {
            Image::Raster(raster) => {
                let mut encoded_out: Option<(u32, u32, String, Vec<u8>)> = if !self.extract_images {
                    let w = raster.width();
                    let h = raster.height();
                    (w > 0 && h > 0).then(|| (w, h, String::new(), Vec::new()))
                } else {
                    None
                };
                if self.extract_images {
                    raster.with_rgba(
                        |img_data, alpha_data| {
                            let w = img_data.width();
                            let h = img_data.height();
                            if w == 0 || h == 0 {
                                return;
                            }
                            match (img_data, alpha_data) {
                                (ImageData::Rgb(rgb), None) => {
                                    let mut buf = Vec::new();
                                    let enc = jpeg_encoder::Encoder::new(&mut buf, 88);
                                    if enc
                                        .encode(
                                            &rgb.data,
                                            w as u16,
                                            h as u16,
                                            jpeg_encoder::ColorType::Rgb,
                                        )
                                        .is_ok()
                                    {
                                        encoded_out = Some((w, h, "jpeg".to_string(), buf));
                                    }
                                }
                                (ImageData::Rgb(rgb), Some(alpha)) => {
                                    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                                    for (px, &a) in rgb.data.chunks_exact(3).zip(alpha.data.iter()) {
                                        rgba.extend_from_slice(&[px[0], px[1], px[2], a]);
                                    }
                                    if let Ok(png_bytes) = encode_rgba_png(w, h, &rgba) {
                                        encoded_out = Some((w, h, "png".to_string(), png_bytes));
                                    }
                                }
                                (ImageData::Luma(luma), _) => {
                                    let mut buf = Vec::new();
                                    let enc = jpeg_encoder::Encoder::new(&mut buf, 88);
                                    if enc
                                        .encode(
                                            &luma.data,
                                            w as u16,
                                            h as u16,
                                            jpeg_encoder::ColorType::Luma,
                                        )
                                        .is_ok()
                                    {
                                        encoded_out = Some((w, h, "jpeg".to_string(), buf));
                                    }
                                }
                            }
                        },
                        None,
                    );
                }
                if let Some((pixel_width, pixel_height, format, data)) = encoded_out {
                    // `props.transform` maps from pixel coordinates `(0..w, 0..h)` into page coordinates
                    let wf = pixel_width as f64;
                    let hf = pixel_height as f64;
                    let p00 = props.transform * Point::new(0.0, 0.0);
                    let p10 = props.transform * Point::new(wf, 0.0);
                    let p01 = props.transform * Point::new(0.0, hf);
                    let p11 = props.transform * Point::new(wf, hf);
                    let min_x = p00.x.min(p10.x).min(p01.x).min(p11.x) as f32;
                    let max_x = p00.x.max(p10.x).max(p01.x).max(p11.x) as f32;
                    let min_y = p00.y.min(p10.y).min(p01.y).min(p11.y) as f32;
                    let max_y = p00.y.max(p10.y).max(p01.y).max(p11.y) as f32;
                    let raw_bbox = RectPt::new(min_x, min_y, max_x, max_y);

                    let active_clip = self
                        .clip_stack
                        .last()
                        .copied()
                        .unwrap_or(self.page_rect)
                        .intersection(&self.page_rect)
                        .unwrap_or(self.page_rect);
                    let Some(bbox) = raw_bbox.intersection(&active_clip) else {
                        return;
                    };
                    if bbox.width() < 4.0 || bbox.height() < 4.0 {
                        return;
                    }

                    let is_full_page_scan = bbox.area() >= 0.70 * self.page_rect.area();
                    self.images.push(ImageIr {
                        bbox,
                        pixel_width,
                        pixel_height,
                        format,
                        data,
                        is_full_page_scan,
                    });
                }
            }
            Image::Stencil(_) => {}
        }
    }
}

fn extract_scale_factor(tf: Affine) -> f32 {
    let c = tf.as_coeffs();
    let sx = libm::hypotf(c[0] as f32, c[1] as f32);
    let sy = libm::hypotf(c[2] as f32, c[3] as f32);
    0.5 * (sx + sy)
}

fn transform_rect(tf: Affine, r: Rect) -> RectPt {
    let p00 = tf * Point::new(r.x0, r.y0);
    let p10 = tf * Point::new(r.x1, r.y0);
    let p01 = tf * Point::new(r.x0, r.y1);
    let p11 = tf * Point::new(r.x1, r.y1);
    let x0 = p00.x.min(p10.x).min(p01.x).min(p11.x) as f32;
    let y0 = p00.y.min(p10.y).min(p01.y).min(p11.y) as f32;
    let x1 = p00.x.max(p10.x).max(p01.x).max(p11.x) as f32;
    let y1 = p00.y.max(p10.y).max(p01.y).max(p11.y) as f32;
    RectPt::new(x0, y0, x1, y1)
}

fn is_axis_aligned_box(elements: &[PathEl]) -> bool {
    if !(4..=6).contains(&elements.len()) {
        return false;
    }
    let mut prev: Option<Point> = None;
    for el in elements {
        match *el {
            PathEl::MoveTo(p) => prev = Some(p),
            PathEl::LineTo(p) => {
                if let Some(pr) = prev {
                    let dx = (p.x - pr.x).abs();
                    let dy = (p.y - pr.y).abs();
                    if dx > 1.0 && dy > 1.0 {
                        return false;
                    }
                }
                prev = Some(p);
            }
            PathEl::ClosePath => {}
            _ => return false,
        }
    }
    true
}

pub fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

// ============================================================================
// Secondary Backend: Zero-Copy Binary Page-Dump & PyMuPDF/PDFium Oracle Loader
// ============================================================================

/// Magic header (`"P2DDMP01"`) for zero-copy flat binary page dumps from `@embedpdf/pdfium` in WASM.
pub const PDFIUM_DUMP_MAGIC: &[u8; 8] = b"P2DDMP01";

/// Decodes a flat binary buffer produced by the JS `@embedpdf/pdfium` page-dump shim
/// directly from WASM linear memory without JSON parsing overhead.
pub fn decode_pdfium_binary_dump(buf: &[u8]) -> Result<DocIr, OpenError> {
    if buf.len() < 12 || &buf[0..8] != PDFIUM_DUMP_MAGIC {
        return Err(OpenError::Corrupt(
            "Invalid PDFium binary page-dump magic".to_string(),
        ));
    }
    let page_count = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
    let mut cursor = 12usize;
    let mut pages = Vec::with_capacity(page_count);

    for page_index in 0..page_count {
        if cursor + 12 > buf.len() {
            return Err(OpenError::Corrupt("Truncated page header".to_string()));
        }
        let width_pt = f32::from_le_bytes([
            buf[cursor],
            buf[cursor + 1],
            buf[cursor + 2],
            buf[cursor + 3],
        ]);
        let height_pt = f32::from_le_bytes([
            buf[cursor + 4],
            buf[cursor + 5],
            buf[cursor + 6],
            buf[cursor + 7],
        ]);
        let glyph_count = u32::from_le_bytes([
            buf[cursor + 8],
            buf[cursor + 9],
            buf[cursor + 10],
            buf[cursor + 11],
        ]) as usize;
        cursor += 12;

        let mut glyphs = Vec::with_capacity(glyph_count);
        for _ in 0..glyph_count {
            // Each binary glyph record is 32 bytes:
            // codepoint: u32, x0: f32, y0: f32, x1: f32, y1: f32, baseline_y: f32, size_pt: f32, flags_rgba: u32
            if cursor + 32 > buf.len() {
                return Err(OpenError::Corrupt("Truncated glyph record".to_string()));
            }
            let cp = u32::from_le_bytes([
                buf[cursor],
                buf[cursor + 1],
                buf[cursor + 2],
                buf[cursor + 3],
            ]);
            let x0 = f32::from_le_bytes([
                buf[cursor + 4],
                buf[cursor + 5],
                buf[cursor + 6],
                buf[cursor + 7],
            ]);
            let y0 = f32::from_le_bytes([
                buf[cursor + 8],
                buf[cursor + 9],
                buf[cursor + 10],
                buf[cursor + 11],
            ]);
            let x1 = f32::from_le_bytes([
                buf[cursor + 12],
                buf[cursor + 13],
                buf[cursor + 14],
                buf[cursor + 15],
            ]);
            let y1 = f32::from_le_bytes([
                buf[cursor + 16],
                buf[cursor + 17],
                buf[cursor + 18],
                buf[cursor + 19],
            ]);
            let baseline_y = f32::from_le_bytes([
                buf[cursor + 20],
                buf[cursor + 21],
                buf[cursor + 22],
                buf[cursor + 23],
            ]);
            let font_size_pt = f32::from_le_bytes([
                buf[cursor + 24],
                buf[cursor + 25],
                buf[cursor + 26],
                buf[cursor + 27],
            ]);
            let r = buf[cursor + 28];
            let g = buf[cursor + 29];
            let b = buf[cursor + 30];
            let style_flags = buf[cursor + 31];
            cursor += 32;

            let unicode = char::from_u32(cp)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "\u{FFFD}".to_string());
            let bbox = RectPt::from_points(x0, y0, x1, y1);
            let font_idx = (style_flags & 0x03) as usize;

            glyphs.push(GlyphIr {
                unicode,
                glyph_id: cp as u16,
                bbox,
                baseline_y,
                font_size_pt,
                advance_pt: bbox.width().max(font_size_pt * 0.25),
                char_spacing_pt: 0.0,
                word_spacing_pt: 0.0,
                horiz_scale: 1.0,
                angle_deg: 0.0,
                font_idx,
                color: RgbaColor { r, g, b, a: 255 },
                render_mode: RenderMode::Fill,
                mcid: None,
                actual_text: None,
                is_visible_in_clip: true,
            });
        }

        pages.push(PageIr {
            page_index,
            width_pt,
            height_pt,
            rotation_deg: 0,
            fonts: vec![
                FontIr {
                    postscript_name: "Helvetica".to_string(),
                    family_name: "Helvetica".to_string(),
                    is_bold: false,
                    is_italic: false,
                    ..Default::default()
                },
                FontIr {
                    postscript_name: "Helvetica-Bold".to_string(),
                    family_name: "Helvetica".to_string(),
                    is_bold: true,
                    is_italic: false,
                    ..Default::default()
                },
                FontIr {
                    postscript_name: "Helvetica-Oblique".to_string(),
                    family_name: "Helvetica".to_string(),
                    is_bold: false,
                    is_italic: true,
                    ..Default::default()
                },
                FontIr {
                    postscript_name: "Helvetica-BoldOblique".to_string(),
                    family_name: "Helvetica".to_string(),
                    is_bold: true,
                    is_italic: true,
                    ..Default::default()
                },
            ],
            glyphs,
            paths: Vec::new(),
            images: Vec::new(),
            annotations: Vec::new(),
            form_fields: Vec::new(),
            page_raster_png: None,
            graphics_png: None,
            graphics_format: String::new(),
            has_complex_graphics: false,
        });
    }

    Ok(DocIr {
        pages,
        has_acroform: false,
        producer: Some("pdfium-dump".to_string()),
        title: None,
    })
}
