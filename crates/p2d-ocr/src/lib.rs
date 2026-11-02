use p2d_ir::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrWordBox {
    pub text: String,
    pub bbox_pt: RectPt,
    pub confidence: f32,
}

pub trait OcrProvider {
    fn name(&self) -> &'static str;
    fn recognize_page(&self, page_index: usize, width_pt: f32, height_pt: f32) -> Vec<OcrWordBox>;
}

/// Default no-op OCR provider when no OCR runtime is attached.
#[derive(Debug, Default, Clone)]
pub struct NoopOcrProvider;

impl OcrProvider for NoopOcrProvider {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn recognize_page(&self, _page_index: usize, _width_pt: f32, _height_pt: f32) -> Vec<OcrWordBox> {
        Vec::new()
    }
}

/// Fast ¼-scale background illumination flattening for phone photos of documents.
/// Downsamples luma by 4x, computes a separable local max envelope, upsamples, and normalizes.
pub fn flatten_illumination_luma(luma: &[u8], width: usize, height: usize) -> Vec<u8> {
    if width < 8 || height < 8 || luma.len() < width * height {
        return luma.to_vec();
    }
    let sw = (width / 4).max(1);
    let sh = (height / 4).max(1);
    let mut small = vec![0u8; sw * sh];
    for sy in 0..sh {
        let y = (sy * 4).min(height - 1);
        for sx in 0..sw {
            let x = (sx * 4).min(width - 1);
            small[sy * sw + sx] = luma[y * width + x];
        }
    }

    // Separable 5-tap max filter on ¼-scale image to estimate paper background illumination
    let radius = 3usize;
    let mut bg_h = vec![0u8; sw * sh];
    for sy in 0..sh {
        for sx in 0..sw {
            let mut max_v = 128u8;
            let x0 = sx.saturating_sub(radius);
            let x1 = (sx + radius).min(sw - 1);
            for nx in x0..=x1 {
                max_v = max_v.max(small[sy * sw + nx]);
            }
            bg_h[sy * sw + sx] = max_v;
        }
    }

    let mut bg = vec![0u8; sw * sh];
    for sy in 0..sh {
        let y0 = sy.saturating_sub(radius);
        let y1 = (sy + radius).min(sh - 1);
        for sx in 0..sw {
            let mut max_v = 128u8;
            for ny in y0..=y1 {
                max_v = max_v.max(bg_h[ny * sw + sx]);
            }
            bg[sy * sw + sx] = max_v.max(32);
        }
    }

    // Normalize full-res luma against upsampled background
    let mut out = vec![0u8; width * height];
    for y in 0..height {
        let sy = (y / 4).min(sh - 1);
        for x in 0..width {
            let sx = (x / 4).min(sw - 1);
            let b = bg[sy * sw + sx] as u32;
            let v = luma[y * width + x] as u32;
            let norm = ((v * 255) / b).min(255) as u8;
            out[y * width + x] = norm;
        }
    }
    out
}

/// Convert recognized `OcrWordBox` items into synthetic `GlyphIr` items so the exact same
/// `p2d-layout` pipeline reconstructs paragraphs, tables, and searchable PDF layers.
pub fn ocr_boxes_to_glyphs(boxes: &[OcrWordBox]) -> Vec<GlyphIr> {
    let mut glyphs = Vec::with_capacity(boxes.len());
    for wb in boxes {
        if wb.text.trim().is_empty() {
            continue;
        }
        let font_size_pt = (wb.bbox_pt.height() * 0.85).clamp(7.0, 48.0);
        glyphs.push(GlyphIr {
            unicode: wb.text.clone(),
            glyph_id: 1,
            bbox: wb.bbox_pt,
            baseline_y: wb.bbox_pt.y1 - font_size_pt * 0.18,
            font_size_pt,
            advance_pt: wb.bbox_pt.width(),
            char_spacing_pt: 0.0,
            word_spacing_pt: 0.0,
            horiz_scale: 1.0,
            angle_deg: 0.0,
            font_idx: 0,
            color: RgbaColor::BLACK,
            render_mode: RenderMode::Fill,
            mcid: None,
            actual_text: None,
            is_visible_in_clip: true,
        });
    }
    glyphs
}
