//! Render-back pixel fidelity gate: convert each PDF, let Apple Pages lay the DOCX out and export it
//! back to PDF, render both PDFs and measure how well their ink overlaps.
//!
//! Score: F1 of "ink" pixels (darker than mid-grey) between the original and the round-tripped page,
//! counting a pixel as matched when the other page has ink within 1.5 px (and within 3 px) at 100 dpi.
//! 1.0 means every mark is where it was.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use p2d_engine::convert_pdf_never_fail;
use p2d_ir::{OutputFormat, Params, Preset};
use p2d_pdf::{HayroBackend, PdfBackend};

use crate::pages_app;

const DPI: f32 = 100.0;
const INK_THRESHOLD: u8 = 170;

pub struct PageScore {
    pub f1_1_5: f64,
    pub f1_3: f64,
}

pub struct DocScore {
    pub name: String,
    pub pages: Vec<PageScore>,
    pub page_count_matches: bool,
    pub error: Option<String>,
}

impl DocScore {
    pub fn mean_f1(&self, tight: bool) -> f64 {
        if self.pages.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.pages.iter().map(|p| if tight { p.f1_1_5 } else { p.f1_3 }).sum();
        sum / self.pages.len() as f64
    }
}

pub fn run_fidelity(pdfs: &[PathBuf], out_dir: &Path, preset: Preset) -> Vec<DocScore> {
    let _ = fs::create_dir_all(out_dir);
    pdfs.iter().map(|pdf| score_document(pdf, out_dir, preset)).collect()
}

fn score_document(pdf: &Path, out_dir: &Path, preset: Preset) -> DocScore {
    let name = pdf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document")
        .to_string();
    let fail = |error: String| DocScore {
        name: name.clone(),
        pages: Vec::new(),
        page_count_matches: false,
        error: Some(error),
    };
    let Ok(original) = fs::read(pdf) else {
        return fail("lecture impossible".into());
    };
    let params = Params {
        preset,
        format: OutputFormat::Docx,
        ..Default::default()
    };
    let docx = match convert_pdf_never_fail(&original, None, &params) {
        Ok((bytes, _, _)) => bytes,
        Err(e) => return fail(format!("conversion: {e}")),
    };
    let docx_path = out_dir.join(format!("{name}.docx"));
    let roundtrip_path = out_dir.join(format!("{name}_pages.pdf"));
    if fs::write(&docx_path, &docx).is_err() {
        return fail("écriture du .docx impossible".into());
    }
    if !pages_app::convert_docx(&docx_path, &roundtrip_path, pages_app::PagesOutput::Pdf) {
        return fail("export Pages impossible (Pages installé ? automatisation autorisée ?)".into());
    }
    let Ok(roundtrip) = fs::read(&roundtrip_path) else {
        return fail("lecture de l’export Pages impossible".into());
    };

    let original: Arc<[u8]> = Arc::from(original);
    let roundtrip: Arc<[u8]> = Arc::from(roundtrip);
    let backend = HayroBackend::new();
    let mut pages = Vec::new();
    let mut page_count_matches = true;
    for index in 0.. {
        let Some(a) = render_gray(&backend, &original, index) else {
            break;
        };
        let Some(b) = render_gray(&backend, &roundtrip, index) else {
            page_count_matches = false;
            pages.push(PageScore { f1_1_5: 0.0, f1_3: 0.0 });
            continue;
        };
        pages.push(compare(&a, &b));
    }
    if render_gray(&backend, &roundtrip, pages.len()).is_some() {
        page_count_matches = false;
    }
    DocScore {
        name,
        pages,
        page_count_matches,
        error: None,
    }
}

struct Gray {
    width: usize,
    height: usize,
    ink: Vec<bool>,
}

fn render_gray(backend: &HayroBackend, pdf: &Arc<[u8]>, index: usize) -> Option<Gray> {
    let png = backend.render_page_png(pdf.clone(), None, index, DPI).ok()?;
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let channels = info.color_type.samples();
    let (width, height) = (info.width as usize, info.height as usize);
    let ink = buf[..info.buffer_size()]
        .chunks(channels)
        .map(|px| {
            let luma =
                (px[0] as u32 * 299 + px[1.min(channels - 1)] as u32 * 587 + px[2.min(channels - 1)] as u32 * 114)
                    / 1000;
            luma < INK_THRESHOLD as u32
        })
        .collect();
    Some(Gray { width, height, ink })
}

/// Ink of `mask` grown by a disc of `radius` pixels.
fn dilate(mask: &Gray, radius: f64) -> Vec<bool> {
    let r = radius.ceil() as i64;
    let offsets: Vec<(i64, i64)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| ((dx * dx + dy * dy) as f64) <= radius * radius)
        .collect();
    let (w, h) = (mask.width as i64, mask.height as i64);
    let mut out = vec![false; mask.ink.len()];
    for y in 0..h {
        for x in 0..w {
            if !mask.ink[(y * w + x) as usize] {
                continue;
            }
            for (dx, dy) in &offsets {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && nx < w && ny < h {
                    out[(ny * w + nx) as usize] = true;
                }
            }
        }
    }
    out
}

fn f1(a: &Gray, b: &Gray, a_grown: &[bool], b_grown: &[bool]) -> f64 {
    let (mut b_hit, mut b_total, mut a_hit, mut a_total) = (0u64, 0u64, 0u64, 0u64);
    for i in 0..a.ink.len().min(b.ink.len()) {
        if b.ink[i] {
            b_total += 1;
            b_hit += a_grown[i] as u64;
        }
        if a.ink[i] {
            a_total += 1;
            a_hit += b_grown[i] as u64;
        }
    }
    if a_total == 0 && b_total == 0 {
        return 1.0;
    }
    if a_total == 0 || b_total == 0 {
        return 0.0;
    }
    let precision = b_hit as f64 / b_total as f64;
    let recall = a_hit as f64 / a_total as f64;
    if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

fn compare(a: &Gray, b: &Gray) -> PageScore {
    if a.width != b.width || a.height != b.height {
        // Different page size: nothing lines up.
        return PageScore { f1_1_5: 0.0, f1_3: 0.0 };
    }
    PageScore {
        f1_1_5: f1(a, b, &dilate(a, 1.5), &dilate(b, 1.5)),
        f1_3: f1(a, b, &dilate(a, 3.0), &dilate(b, 3.0)),
    }
}
