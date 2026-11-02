use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, OnceLock};

use p2d_engine::{EditablePdf, LineEdit, convert_pdf_never_fail, inspect_pdf_bytes, open_for_editing, save_line_edits};
use p2d_ir::{OutputFormat, Params, Preset, Profile, RgbaColor};
use p2d_layout::exact::EditableLine;
use p2d_layout::fonts::{
    FontMetrics, NoFontMetrics, pdf_font_family_candidates, pdf_font_weight_subfamily, register_memory_font,
};
use p2d_pdf::{ExtractOptions, HayroBackend, OpenError, PdfBackend};
use serde::Serialize;
use wasm_bindgen::prelude::*;

struct WasmEditState {
    filename: String,
    pdf: EditablePdf,
}

fn active_edit_state() -> &'static Mutex<Option<WasmEditState>> {
    static STATE: OnceLock<Mutex<Option<WasmEditState>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

#[derive(Serialize)]
struct EditPageView {
    width_pt: f32,
    height_pt: f32,
    lines: Vec<EditLineView>,
}

#[derive(Serialize)]
struct EditLineView {
    #[serde(flatten)]
    line: EditableLine,
    background: RgbaColor,
    run_fonts: Vec<Option<String>>,
}

#[derive(Serialize)]
struct EditFontView {
    family: String,
    url: String,
    bold: bool,
    italic: bool,
    size_adjust_pct: f32,
}

fn build_edit_view_json(state: &WasmEditState) -> serde_json::Value {
    let pages: Vec<EditPageView> = state
        .pdf
        .doc
        .pages
        .iter()
        .zip(&state.pdf.pages)
        .enumerate()
        .map(|(i, (page, lines))| {
            let colors = state
                .pdf
                .backgrounds
                .get(i)
                .cloned()
                .unwrap_or_else(|| vec![RgbaColor::WHITE; lines.len()]);
            EditPageView {
                width_pt: page.width_pt,
                height_pt: page.height_pt,
                lines: lines
                    .iter()
                    .cloned()
                    .zip(colors)
                    .map(|(line, background)| EditLineView {
                        run_fonts: line
                            .runs
                            .iter()
                            .map(|run| state.pdf.run_font(i, run).map(|f| state.pdf.fonts[f].family.clone()))
                            .collect(),
                        line,
                        background,
                    })
                    .collect(),
            }
        })
        .collect();
    let fonts: Vec<EditFontView> = state
        .pdf
        .fonts
        .iter()
        .enumerate()
        .map(|(i, f)| EditFontView {
            family: f.family.clone(),
            url: format!("wasm-font:{i}"),
            bold: f.bold,
            italic: f.italic,
            size_adjust_pct: 100.0 / f.size_scale.max(0.05),
        })
        .collect();
    serde_json::json!({
        "ok": true,
        "session": "wasm",
        "filename": state.filename,
        "pages": pages,
        "fonts": fonts,
    })
}

/// Inspect a PDF inside the browser or Chrome MV3 Offscreen Worker.
/// Returns a JSON string with `{ needs_password, bad_password, is_corrupt, page_count, has_acroform, form_field_count, primary_case }`.
#[wasm_bindgen]
pub fn wasm_inspect_pdf(pdf_bytes: &[u8], password: Option<String>) -> String {
    let info = inspect_pdf_bytes(pdf_bytes, password.as_deref());
    serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string())
}

/// Register a TrueType/OpenType `.ttf` font in WASM memory so `Preset::Exact` and the in-place
/// PDF editor can measure its exact glyph advances/kerning and embed missing characters.
#[wasm_bindgen]
pub fn wasm_register_font(family: &str, bold: bool, italic: bool, ttf_bytes: &[u8]) {
    register_memory_font(family, bold, italic, ttf_bytes);
}

/// Returns a JSON array of font family names used by `pdf_bytes` that are not yet registered in
/// WASM memory, so the browser can fetch them from Google Fonts / Fontsource CDN before converting.
#[wasm_bindgen]
pub fn wasm_missing_font_families(pdf_bytes: &[u8], password: Option<String>) -> String {
    let opts = ExtractOptions {
        extract_images: false,
        render_fallback_png: false,
        fallback_dpi: 72.0,
        render_graphics_dpi: None,
    };
    let Ok((doc, _)) = HayroBackend::new().extract_document(Arc::from(pdf_bytes), password.as_deref(), &opts) else {
        return "[]".to_string();
    };
    let metrics = NoFontMetrics;
    let mut missing = BTreeSet::new();
    for page in &doc.pages {
        for font in &page.fonts {
            if let Some(cand) = pdf_font_family_candidates(font).into_iter().next() {
                if !cand.is_empty() {
                    if !metrics.has_family(&cand) {
                        missing.insert(cand.clone());
                    }
                    if let Some(sub) = pdf_font_weight_subfamily(font) {
                        let sub_family = format!("{cand} {sub}");
                        if !metrics.has_family(&sub_family) {
                            missing.insert(sub_family);
                        }
                    }
                }
            }
        }
    }
    serde_json::to_string(&missing).unwrap_or_else(|_| "[]".to_string())
}

/// Convert a PDF buffer (or `P2DDMP01` binary page dump) to `.docx` (or `.pages` / `.html` / `.md` / `.txt` / `.pdf`).
///
/// - `format_str`: `"docx"` (default), `"pages"`, `"html"`, `"md"`, `"txt"`, `"spdf"`
/// - `profile_str`: `"pages"` (default), `"portable"`, `"word"`, `"libreoffice"`
/// - `preset_str`: `"exact"`, `"editable"` (default), `"fidele"`
#[wasm_bindgen]
pub fn wasm_convert_pdf(
    pdf_bytes: &[u8],
    password: Option<String>,
    format_str: &str,
    profile_str: &str,
    preset_str: &str,
) -> Result<Vec<u8>, JsValue> {
    let format = match format_str {
        "pages" => OutputFormat::Pages,
        "html" => OutputFormat::Html,
        "md" | "markdown" => OutputFormat::Markdown,
        "txt" | "text" => OutputFormat::Txt,
        "spdf" | "pdf" => OutputFormat::SearchablePdf,
        _ => OutputFormat::Docx,
    };
    let profile = match profile_str {
        "word" => Profile::Word,
        "libreoffice" => Profile::LibreOffice,
        "portable" => Profile::Portable,
        _ => Profile::Pages,
    };
    let preset = match preset_str {
        "exact" => Preset::Exact,
        "fidele" => Preset::Fidele,
        _ => Preset::Editable,
    };

    // In online WASM mode, `OutputFormat::Pages` emits a Pages-tuned `.docx` (which Apple Pages
    // opens natively with 1:1 anchored text boxes and background graphics): the browser has no
    // Pages template to fill in.
    let effective_format = if format == OutputFormat::Pages {
        OutputFormat::Docx
    } else {
        format
    };

    let params = Params {
        profile,
        preset,
        format: effective_format,
        ..Default::default()
    };

    match convert_pdf_never_fail(pdf_bytes, password.as_deref(), &params) {
        Ok((bytes, _flow_doc, _report)) => Ok(bytes),
        Err(e) => Err(JsValue::from_str(&e.to_string())),
    }
}

/// Convert a PDF buffer and return a JSON conversion report alongside the last conversion.
#[wasm_bindgen]
pub fn wasm_convert_report_json(
    pdf_bytes: &[u8],
    password: Option<String>,
) -> Result<String, JsValue> {
    let params = Params::default();
    match convert_pdf_never_fail(pdf_bytes, password.as_deref(), &params) {
        Ok((_bytes, _flow_doc, report)) => {
            Ok(serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string()))
        }
        Err(e) => Err(JsValue::from_str(&e.to_string())),
    }
}

/// Open a PDF in WASM memory for the Direct In-Place 1:1 PDF Editor (`edit.html`).
/// Returns the JSON view payload (`{ ok, session: "wasm", filename, pages, fonts }`).
#[wasm_bindgen]
pub fn wasm_edit_open(filename: &str, pdf_bytes: &[u8], password: Option<String>) -> String {
    let pdf = match open_for_editing(pdf_bytes, password.as_deref()) {
        Ok(pdf) => pdf,
        Err(OpenError::NeedsPassword) => {
            return serde_json::json!({
                "ok": false,
                "needs_password": true,
                "bad_password": password.as_ref().is_some_and(|p| !p.is_empty()),
                "error": "Ce PDF est protégé par un mot de passe."
            })
            .to_string();
        }
        Err(OpenError::BadPassword) => {
            return serde_json::json!({
                "ok": false,
                "needs_password": true,
                "bad_password": true,
                "error": "Le mot de passe saisi est incorrect."
            })
            .to_string();
        }
        Err(e) => {
            return serde_json::json!({ "ok": false, "error": e.to_string() }).to_string();
        }
    };
    if let Err(p2d_pdf::edit::EditError::Encrypted) = save_line_edits(&pdf, &[]) {
        return serde_json::json!({
            "ok": false,
            "error": "Ce PDF est protégé contre les modifications : il ne peut pas être corrigé directement. Revenez en arrière et choisissez « Modifier dans Pages » ou « Modifier dans Word »."
        })
        .to_string();
    }
    let state = WasmEditState {
        filename: filename.to_string(),
        pdf,
    };
    let view = build_edit_view_json(&state);
    if let Ok(mut guard) = active_edit_state().lock() {
        *guard = Some(state);
    }
    view.to_string()
}

/// Returns the current WASM editor view JSON for the active session.
#[wasm_bindgen]
pub fn wasm_edit_view_json() -> String {
    let Ok(guard) = active_edit_state().lock() else {
        return serde_json::json!({ "ok": false, "error": "Session introuvable." }).to_string();
    };
    match guard.as_ref() {
        Some(state) => build_edit_view_json(state).to_string(),
        None => serde_json::json!({ "ok": false, "error": "Aucun document ouvert." }).to_string(),
    }
}

/// Returns the synthesized/rebuilt OpenType `.ttf` bytes for font `font_index` of the active
/// editing session, so the browser can load it via `FontFace`.
#[wasm_bindgen]
pub fn wasm_edit_font_bytes(font_index: usize) -> Option<Vec<u8>> {
    let guard = active_edit_state().lock().ok()?;
    let state = guard.as_ref()?;
    state.pdf.fonts.get(font_index)?.web_font()
}

/// Renders `page_index` of the active editing session to PNG bytes at `dpi` (e.g. 150.0).
#[wasm_bindgen]
pub fn wasm_edit_render_page(page_index: usize, dpi: f32) -> Result<Vec<u8>, JsValue> {
    let bytes = {
        let guard = active_edit_state()
            .lock()
            .map_err(|_| JsValue::from_str("Session verrouillée"))?;
        let state = guard
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Aucun document ouvert"))?;
        state.pdf.pdf_bytes.clone()
    };
    HayroBackend::new()
        .render_page_png(bytes, None, page_index, dpi.clamp(72.0, 220.0))
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Applies `edits_json` (`Vec<LineEdit>`) in memory and renders `page_index` of the modified PDF
/// to PNG bytes at `dpi`, so the live preview in `edit.html` is 100% identical to the saved PDF.
#[wasm_bindgen]
pub fn wasm_edit_preview_page(page_index: usize, edits_json: &str, dpi: f32) -> Result<Vec<u8>, JsValue> {
    let edits: Vec<LineEdit> =
        serde_json::from_str(edits_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let modified_bytes: Arc<[u8]> = {
        let guard = active_edit_state()
            .lock()
            .map_err(|_| JsValue::from_str("Session verrouillée"))?;
        let state = guard
            .as_ref()
            .ok_or_else(|| JsValue::from_str("Aucun document ouvert"))?;
        if edits.is_empty() {
            state.pdf.pdf_bytes.clone()
        } else {
            let saved = save_line_edits(&state.pdf, &edits)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            Arc::from(saved.pdf)
        }
    };
    HayroBackend::new()
        .render_page_png(modified_bytes, None, page_index, dpi.clamp(72.0, 220.0))
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Applies `edits_json` (`Vec<LineEdit>`) to the active PDF, updates the active session to the
/// newly saved PDF (so further edits stack cleanly), and returns the saved PDF bytes.
#[wasm_bindgen]
pub fn wasm_edit_save(edits_json: &str) -> Result<Vec<u8>, JsValue> {
    let edits: Vec<LineEdit> =
        serde_json::from_str(edits_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let mut guard = active_edit_state()
        .lock()
        .map_err(|_| JsValue::from_str("Session verrouillée"))?;
    let state = guard
        .as_mut()
        .ok_or_else(|| JsValue::from_str("Aucun document ouvert"))?;
    let bytes = save_line_edits(&state.pdf, &edits)
        .map_err(|e| JsValue::from_str(&e.to_string()))?
        .pdf;
    if let Ok(reopened) = open_for_editing(&bytes, None) {
        state.pdf = reopened;
    }
    Ok(bytes)
}
