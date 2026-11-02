//! The app's pages and local API, behind two front doors that both call `respond`: the local web
//! server of the developer's Mac (`p2d-cli app`, used in a browser) and the Mac app's own window
//! (`app_window`, through its private `app://` address).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};

use p2d_engine::convert_pdf_never_fail;
use p2d_ir::{OutputFormat, Params, Preset, Profile};
use p2d_pdf::{HayroBackend, PdfBackend};
use serde::{Deserialize, Serialize};

/// A request from the page, whichever way it came in.
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: Vec<u8>,
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn new(status: u16, content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self { status, content_type, body: body.into() }
    }

    fn json(value: &impl Serialize) -> Self {
        Self::new(200, "application/json", serde_json::to_vec(value).unwrap_or_default())
    }

    fn not_found(text: &str) -> Self {
        Self::new(404, "text/plain; charset=utf-8", text.as_bytes())
    }
}

/// What the two front doors do differently.
pub struct Host {
    /// Read the pages from `extension/` when they are on disk (the developer's server: a reload
    /// shows the edits) rather than the copies built into the program.
    pub pages_from_disk: bool,
    /// Offer the QA examples of the developer's Mac (`qa/corpus`, Dropbox).
    pub samples: bool,
    /// Show the Mac's folder dialog, opened in the given folder.
    pub ask_folder: Box<dyn Fn(&Path) -> Option<PathBuf> + Send + Sync>,
}

#[derive(Deserialize)]
struct ConvertRequest {
    filename: String,
    #[serde(default)]
    base64_pdf: Option<String>,
    #[serde(default)]
    sample_file: Option<String>,
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    open_in_pages: bool,
    /// Where to save: "desktop", "downloads" or "chosen" (see `places::dir`).
    #[serde(default)]
    place: Option<String>,
    /// Id the page asks `/api/progress` about while it waits.
    #[serde(default)]
    job: Option<String>,
}

#[derive(Serialize)]
struct ConvertResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default)]
    needs_password: bool,
    #[serde(default)]
    bad_password: bool,
    /// The document was made but could not be written where the person chose (see `write_error`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    save_failed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<p2d_ir::ConvertReport>,
}

/// The data behind `mutex`, even when a request panicked while holding it: that request got an
/// error message (see `respond`) and the app goes on.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// PDFs opened in the direct editor, by session id (the app serves one person at a time).
fn edit_sessions() -> &'static Mutex<HashMap<String, EditSession>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, EditSession>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Editor sessions kept in memory: the PDF being corrected and the few opened just before it.
const KEPT_EDIT_SESSIONS: usize = 4;

/// The file each editor session last saved: saving again replaces it rather than adding a copy.
fn edit_saved_paths() -> &'static Mutex<HashMap<String, PathBuf>> {
    static SAVED: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    SAVED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Every file this app wrote: the only ones `/api/action` opens or shows.
fn saved_files() -> &'static Mutex<HashSet<PathBuf>> {
    static SAVED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    SAVED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Where each conversion or opening is, by the job id its page sent (`/api/progress`).
fn progress_jobs() -> &'static Mutex<HashMap<String, serde_json::Value>> {
    static JOBS: OnceLock<Mutex<HashMap<String, serde_json::Value>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Run `work` with its steps shown to the page waiting on `job`: the engine's (see
/// `p2d_ir::progress`) and the ones `work` tells with the function it is given.
fn with_progress<T>(job: Option<&str>, work: impl FnOnce(&dyn Fn(serde_json::Value)) -> T) -> T {
    let Some(job) = job.map(str::to_owned) else {
        return work(&|_| {});
    };
    let tell = {
        let job = job.clone();
        move |step: serde_json::Value| {
            lock(progress_jobs()).insert(job.clone(), step);
        }
    };
    // Forgotten when done, even after a panic.
    struct Forget(String);
    impl Drop for Forget {
        fn drop(&mut self) {
            lock(progress_jobs()).remove(&self.0);
        }
    }
    let _forget = Forget(job);
    let engine_tell = tell.clone();
    let sink: p2d_ir::progress::Sink =
        Arc::new(move |step| engine_tell(serde_json::to_value(step).unwrap_or_default()));
    p2d_ir::progress::with_sink(sink, || work(&tell))
}

/// A PDF the Mac sent to the app (« Ouvrir avec », dropped on its Dock icon, ⌘O), until the page
/// takes it.
fn opened_pdf() -> &'static Mutex<Option<(String, Vec<u8>)>> {
    static OPENED: OnceLock<Mutex<Option<(String, Vec<u8>)>>> = OnceLock::new();
    OPENED.get_or_init(|| Mutex::new(None))
}

/// Hand a PDF to the page, which asks for it at `/api/opened-pdf`. A newer one replaces it.
pub fn queue_opened_pdf(name: String, bytes: Vec<u8>) {
    *lock(opened_pdf()) = Some((name, bytes));
}

/// Run `render` when one of the two page-render slots is free. The editor asks for all the pages of
/// a document at once, and each render holds the whole PDF and a page bitmap: a long scan rendered
/// all together would take gigabytes.
fn with_render_slot<T>(render: impl FnOnce() -> T) -> T {
    static SLOTS: OnceLock<(Mutex<usize>, Condvar)> = OnceLock::new();
    let (busy, freed) = SLOTS.get_or_init(|| (Mutex::new(0), Condvar::new()));
    {
        let mut busy = lock(busy);
        while *busy >= 2 {
            busy = freed.wait(busy).unwrap_or_else(PoisonError::into_inner);
        }
        *busy += 1;
    }
    // Freed even if the render panics (`respond` catches it).
    struct Slot<'a>(&'a Mutex<usize>, &'a Condvar);
    impl Drop for Slot<'_> {
        fn drop(&mut self) {
            *lock(self.0) -= 1;
            self.1.notify_one();
        }
    }
    let _slot = Slot(busy, freed);
    render()
}

/// A folder as the page shows it: its name, one part per level (« Documents › Factures »).
fn place_json(folder: &Path) -> serde_json::Value {
    serde_json::json!({ "parts": crate::places::label(folder) })
}

struct EditSession {
    filename: String,
    pdf: p2d_engine::EditablePdf,
    /// Web fonts already built, by index into `EditablePdf::fonts`.
    web_fonts: HashMap<usize, Option<Vec<u8>>>,
    /// The PDF with the pending (not yet saved) edits applied, shown in place of the original.
    preview: Option<std::sync::Arc<[u8]>>,
}

#[derive(Deserialize)]
struct EditOpenRequest {
    filename: String,
    #[serde(default)]
    base64_pdf: Option<String>,
    #[serde(default)]
    sample_file: Option<String>,
    #[serde(default)]
    password: Option<String>,
    /// Id the page asks `/api/progress` about while it waits.
    #[serde(default)]
    job: Option<String>,
}

#[derive(Serialize)]
struct EditPage {
    width_pt: f32,
    height_pt: f32,
    lines: Vec<EditLineView>,
}

#[derive(Serialize)]
struct EditLineView {
    #[serde(flatten)]
    line: p2d_layout::exact::EditableLine,
    /// Colour behind the line, used to hide the original text while it is retyped.
    background: p2d_ir::RgbaColor,
    /// CSS family of the PDF's own font for each run of the line (see `EditFontView`), when it is
    /// embedded.
    run_fonts: Vec<Option<String>>,
}

/// An embedded font the edit page loads with `@font-face`.
#[derive(Serialize)]
struct EditFontView {
    family: String,
    url: String,
    bold: bool,
    italic: bool,
    /// CSS `size-adjust`: the PDF draws these glyphs at a different size than the one it sets.
    size_adjust_pct: f32,
}

#[derive(Deserialize)]
struct EditSaveRequest {
    session: String,
    edits: Vec<p2d_engine::LineEdit>,
    /// Where to save: "desktop", "downloads" or "chosen" (see `places::dir`).
    #[serde(default)]
    place: Option<String>,
}

/// Pending edits to show on the page images (`/api/edit/preview`).
#[derive(Deserialize)]
struct EditPreviewRequest {
    session: String,
    edits: Vec<p2d_engine::LineEdit>,
}

#[derive(Deserialize)]
struct ActionRequest {
    saved_path: String,
    action: String, // "pages" | "word" | "finder" | "open"
}

pub fn run_visual_server(port: u16, auto_open_browser: bool) -> Result<(), String> {
    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr)
        .map_err(|e| format!("Failed to bind visual app server on {addr}: {e}"))?;

    let url = format!("http://{addr}");
    println!("==============================================================");
    println!("  PDF Edit — Visual App running at: {url}");
    println!("==============================================================");

    if auto_open_browser {
        let _ = Command::new("open").arg(&url).spawn();
    }

    let host = Arc::new(Host {
        pages_from_disk: true,
        samples: true,
        ask_folder: Box::new(crate::places::ask_folder_with_osascript),
    });
    for stream in listener.incoming().flatten() {
        let host = Arc::clone(&host);
        // One thread per request: a conversion or the folder dialog does not hold up the others.
        std::thread::spawn(move || {
            let _ = handle_client(stream, port, &host);
        });
    }
    Ok(())
}

/// Largest request body read: a PDF of about 190 MB once in Base64.
const MAX_BODY: usize = 256 << 20;

fn handle_client(mut stream: TcpStream, port: u16, host: &Host) -> Result<(), String> {
    let mut buf = vec![0u8; 65536];
    let mut total_read = 0usize;
    let header_end;

    loop {
        let n = stream.read(&mut buf[total_read..]).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        total_read += n;
        if let Some(pos) = find_subslice(&buf[..total_read], b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if total_read == buf.len() {
            if buf.len() >= 1 << 20 {
                return Ok(());
            }
            buf.resize(buf.len() * 2, 0);
        }
    }

    let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = header_str.lines();
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let raw_path = parts[1];
    let (path, query_str) = raw_path.split_once('?').unwrap_or((raw_path, ""));

    let mut content_length = 0usize;
    let (mut host_header, mut origin, mut fetch_site) = (None, None, None);
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim(), v.trim());
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.parse::<usize>().unwrap_or(0);
            } else if k.eq_ignore_ascii_case("host") {
                host_header = Some(v);
            } else if k.eq_ignore_ascii_case("origin") {
                origin = Some(v);
            } else if k.eq_ignore_ascii_case("sec-fetch-site") {
                fetch_site = Some(v);
            }
        }
    }

    if !from_own_page(port, host_header, origin, fetch_site) {
        return write_response(&mut stream, &Response::new(403, "text/plain; charset=utf-8", "Accès refusé"));
    }
    if content_length > MAX_BODY {
        return write_response(&mut stream, &Response::new(413, "text/plain; charset=utf-8", "Fichier trop gros"));
    }

    let mut body = Vec::with_capacity(content_length);
    body.extend_from_slice(&buf[header_end..total_read]);
    while body.len() < content_length {
        let mut chunk = vec![0u8; (content_length - body.len()).min(65536)];
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }

    let req = Request {
        method: method.to_string(),
        path: path.to_string(),
        query: query_str.to_string(),
        body,
    };
    write_response(&mut stream, &respond(&req, host))
}

/// Whether a request to the local web server comes from its own pages. Any web page open in any
/// browser can send requests to 127.0.0.1: those carry another `Origin` or `Sec-Fetch-Site`, and a
/// site that points its own name at 127.0.0.1 (DNS rebinding) sends that name as `Host`.
fn from_own_page(port: u16, host: Option<&str>, origin: Option<&str>, fetch_site: Option<&str>) -> bool {
    let own = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let host_ok = host.is_some_and(|h| own.iter().any(|o| o.eq_ignore_ascii_case(h)));
    let origin_ok = origin.is_none_or(|o| own.iter().any(|own| o.eq_ignore_ascii_case(&format!("http://{own}"))));
    let site_ok = !fetch_site.is_some_and(|s| s.eq_ignore_ascii_case("cross-site") || s.eq_ignore_ascii_case("same-site"));
    host_ok && origin_ok && site_ok
}

/// The answer to one request. A panic while answering (a PDF the engine chokes on) becomes an
/// error message for the page, and the app goes on.
pub fn respond(req: &Request, host: &Host) -> Response {
    catch_unwind(AssertUnwindSafe(|| route(req, host))).unwrap_or_else(|_| {
        let mut resp = Response::json(&serde_json::json!({
            "ok": false,
            "error": "Une erreur inattendue est survenue avec ce PDF. Réessayez, ou choisissez un autre PDF."
        }));
        resp.status = 500;
        resp
    })
}

fn route(req: &Request, host: &Host) -> Response {
    let (method, path, query_str, body) = (req.method.as_str(), req.path.as_str(), req.query.as_str(), &req.body[..]);

    if method == "GET" && path == "/api/samples" && host.samples {
        let manifest_path = Path::new("qa/corpus/synthetic/manifest.json");
        if !manifest_path.exists() {
            let _ = crate::corpus::generate_corpus(Path::new("qa/corpus/synthetic"));
        }
        let json = fs::read_to_string(manifest_path).unwrap_or_else(|_| "{\"entries\":[]}".to_string());
        return Response::new(200, "application/json; charset=utf-8", json);
    }

    if method == "GET" && path == "/api/pdf" && host.samples {
        let sample_param = query_str
            .split('&')
            .find_map(|kv| kv.strip_prefix("sample="))
            .unwrap_or("01_lettre_assurance.pdf");
        let decoded = url_decode(sample_param);
        let sample_path = resolve_sample_pdf_path(&decoded);
        if let Ok(pdf_bytes) = fs::read(&sample_path) {
            return Response::new(200, "application/pdf", pdf_bytes);
        }
        return Response::not_found("PDF introuvable");
    }

    // A PDF the Mac sent to the app, waiting for the page. `peek=1` (the editor) only asks whether
    // one is waiting, and leaves it for the home page.
    if method == "GET" && path == "/api/opened-pdf" {
        if query_param(query_str, "peek").is_some() {
            return Response::json(&serde_json::json!({ "ok": true, "waiting": lock(opened_pdf()).is_some() }));
        }
        let resp = match lock(opened_pdf()).take() {
            Some((name, bytes)) => serde_json::json!({ "ok": true, "name": name, "base64": encode_base64(&bytes) }),
            None => serde_json::json!({ "ok": false }),
        };
        return Response::json(&resp);
    }

    // What this Mac offers: the « Modifier dans Pages » choice only shows when Pages is there.
    if method == "GET" && path == "/api/app-info" {
        return Response::json(&serde_json::json!({ "ok": true, "pages": crate::pages_app::pages_bundle().is_some() }));
    }

    // The folder picked last time, offered again next to Desktop and Downloads.
    if method == "GET" && path == "/api/save-places" {
        let chosen = crate::places::last_folder().map(|folder| place_json(&folder));
        return Response::json(&serde_json::json!({ "ok": true, "chosen": chosen }));
    }

    // Shows the Mac's folder dialog and waits for the person's answer.
    if method == "POST" && path == "/api/choose-folder" {
        let resp = match crate::places::choose_folder(&*host.ask_folder) {
            Some(folder) => serde_json::json!({ "ok": true, "chosen": place_json(&folder) }),
            None => serde_json::json!({ "ok": false, "canceled": true }),
        };
        return Response::json(&resp);
    }

    if method == "POST" && path == "/api/convert" {
        return convert(body, host);
    }

    // The step a conversion or an opening is at, while its page waits (`null` once it is over).
    if method == "GET" && path == "/api/progress" {
        let job = query_param(query_str, "job").unwrap_or_default();
        let progress = lock(progress_jobs()).get(&job).cloned();
        return Response::json(&serde_json::json!({ "ok": true, "progress": progress }));
    }

    if method == "POST" && path == "/api/edit/open" {
        let resp = match serde_json::from_slice::<EditOpenRequest>(body) {
            Ok(req) => {
                let job = req.job.clone();
                with_progress(job.as_deref(), |_| open_edit_session(req, host))
            }
            Err(e) => serde_json::json!({ "ok": false, "error": format!("Requête invalide : {e}") }),
        };
        return Response::json(&resp);
    }

    if method == "GET" && path == "/api/edit/view" {
        let session = query_param(query_str, "session").unwrap_or_default();
        let resp = lock(edit_sessions())
            .get(&session)
            .map(|s| edit_session_view(&session, s))
            .unwrap_or_else(|| serde_json::json!({ "ok": false, "error": "Ce document n’est plus ouvert : revenez à l’accueil et rechargez-le." }));
        return Response::json(&resp);
    }

    if method == "GET" && path == "/api/edit/page.png" {
        let session = query_param(query_str, "session").unwrap_or_default();
        let page_index = query_param(query_str, "page").and_then(|p| p.parse::<usize>().ok()).unwrap_or(0);
        let with_edits = query_param(query_str, "preview").is_some_and(|v| v == "1");
        // Rendered outside the lock, so the pages of a document render side by side.
        let pdf = lock(edit_sessions()).get(&session).map(|s| match &s.preview {
            Some(preview) if with_edits => preview.clone(),
            _ => s.pdf.pdf_bytes.clone(),
        });
        let bytes = pdf.and_then(|pdf| with_render_slot(|| HayroBackend::new().render_page_png(pdf, None, page_index, 144.0).ok()));
        return match bytes {
            Some(png) => Response::new(200, "image/png", png),
            None => Response::not_found("Page introuvable"),
        };
    }

    if method == "GET" && path == "/api/edit/font" {
        let session = query_param(query_str, "session").unwrap_or_default();
        let font = query_param(query_str, "font").and_then(|p| p.parse::<usize>().ok());
        let bytes = {
            let mut sessions = lock(edit_sessions());
            sessions.get_mut(&session).and_then(|s| {
                let font = font?;
                let built = s
                    .web_fonts
                    .entry(font)
                    .or_insert_with(|| s.pdf.fonts.get(font).and_then(|f| f.web_font()));
                built.clone()
            })
        };
        return match bytes {
            Some(font) => {
                let mime = if font.starts_with(b"OTTO") { "font/otf" } else { "font/ttf" };
                Response::new(200, mime, font)
            }
            None => Response::not_found("Police indisponible"),
        };
    }

    if method == "POST" && path == "/api/edit/preview" {
        let resp = match serde_json::from_slice::<EditPreviewRequest>(body) {
            Ok(req) => preview_edit_session(req),
            Err(e) => serde_json::json!({ "ok": false, "error": format!("Requête invalide : {e}") }),
        };
        return Response::json(&resp);
    }

    if method == "POST" && path == "/api/edit/save" {
        let resp = match serde_json::from_slice::<EditSaveRequest>(body) {
            Ok(req) => save_edit_session(req),
            Err(e) => serde_json::json!({ "ok": false, "error": format!("Requête invalide : {e}") }),
        };
        return Response::json(&resp);
    }

    if method == "POST" && path == "/api/action" {
        if let Ok(req) = serde_json::from_slice::<ActionRequest>(body) {
            let p = PathBuf::from(&req.saved_path);
            // Only a file this app wrote: never any path a request names.
            if lock(saved_files()).contains(&p) && p.exists() {
                let _ = match req.action.as_str() {
                    "finder" => open_file_on_mac(&p, true),
                    // The app that owns `.docx` on this Mac (Word when installed), not Pages.
                    "word" => Command::new("open").arg(&p).spawn().map(|_| ()).map_err(|e| e.to_string()),
                    _ => open_file_on_mac(&p, false),
                };
                return Response::new(200, "application/json", "{\"ok\":true}");
            }
        }
        return Response::new(404, "application/json", "{\"ok\":false,\"error\":\"Fichier introuvable\"}");
    }

    if method == "GET" {
        if let Some((bytes, mime)) = page_file(path, host) {
            return Response::new(200, mime, bytes);
        }
    }
    Response::not_found("Not Found")
}

/// A page file: from `extension/ui` (or `extension/`) on disk for the developer's server, else the
/// copy built into the program. Only plain relative names: never `..` or an absolute path.
fn page_file(path: &str, host: &Host) -> Option<(Vec<u8>, &'static str)> {
    let name = match path {
        "/" | "/index.html" => "drop.html",
        other => other.trim_start_matches('/'),
    };
    let rel = Path::new(name);
    if name.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let mime = crate::assets::mime(name);
    if host.pages_from_disk {
        for dir in ["extension/ui", "extension", "web"] {
            let file = Path::new(dir).join(rel);
            if file.is_file() {
                return fs::read(&file).ok().map(|bytes| (bytes, mime));
            }
        }
    }
    crate::assets::get(name).map(|bytes| (bytes.to_vec(), mime))
}

/// The PDF a request sends: its bytes in Base64, or one of the developer's examples.
fn source_pdf(sample: Option<&str>, base64: Option<&str>, host: &Host) -> Result<Vec<u8>, String> {
    match (sample, base64) {
        (Some(sample), _) if host.samples => {
            fs::read(resolve_sample_pdf_path(sample)).map_err(|e| format!("Impossible de lire l'exemple : {e}"))
        }
        (Some(_), _) => Err("Les exemples ne sont pas disponibles dans cette version.".to_string()),
        (None, Some(b64)) => decode_base64(b64),
        (None, None) => Ok(Vec::new()),
    }
}

/// Why the new file could not be written, in words the person can act on. Refused access is the
/// usual case: the Mac asks once whether the app may use the Desktop or Downloads, and « Refuser »
/// sticks.
fn write_error(folder: &Path, e: &std::io::Error) -> String {
    let name = crate::places::label(folder).join(" › ");
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => format!(
            "Le Mac n’autorise pas PDF Edit à enregistrer dans « {name} ». Choisissez un autre endroit, ou autorisez-le dans Réglages Système › Confidentialité et sécurité › Fichiers et dossiers."
        ),
        std::io::ErrorKind::StorageFull => {
            format!("Il n’y a plus assez de place sur le disque pour enregistrer dans « {name} ».")
        }
        _ => format!("Le nouveau fichier n’a pas pu être enregistré dans « {name} » ({e})."),
    }
}

fn convert_error(error: String, needs_password: bool, bad_password: bool) -> Response {
    Response::json(&ConvertResponse {
        ok: false,
        error: Some(error),
        needs_password,
        bad_password,
        save_failed: false,
        saved_path: None,
        filename: None,
        report: None,
    })
}

fn convert(body: &[u8], host: &Host) -> Response {
    let req: ConvertRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => {
            let mut resp = convert_error(format!("Requête invalide : {e}"), false, false);
            resp.status = 400;
            return resp;
        }
    };

    let pdf_bytes = match source_pdf(req.sample_file.as_deref(), req.base64_pdf.as_deref(), host) {
        Ok(bytes) => bytes,
        Err(e) => return convert_error(e, false, false),
    };

    let preset = match req.preset.as_deref() {
        Some("exact") => Preset::Exact,
        Some("fidele") => Preset::Fidele,
        _ => Preset::Editable,
    };
    let profile = match req.profile.as_deref() {
        Some("word") => Profile::Word,
        _ => Profile::Pages,
    };
    let (format, ext) = match req.format.as_deref() {
        Some("pages") => (OutputFormat::Pages, "pages"),
        Some("html") => (OutputFormat::Html, "html"),
        Some("md") => (OutputFormat::Markdown, "md"),
        Some("txt") => (OutputFormat::Txt, "txt"),
        Some("spdf") => (OutputFormat::SearchablePdf, "pdf"),
        _ => (OutputFormat::Docx, "docx"),
    };

    // Pages itself saves a DOCX (kept out of sight) as `.pages`, so the engine makes that DOCX: the
    // native `.pages` writer only knows flowing text and needs a template only some Pages have.
    let via_pages = format == OutputFormat::Pages;
    let params = Params {
        preset,
        profile,
        format: if via_pages { OutputFormat::Docx } else { format },
        ..Default::default()
    };

    with_progress(req.job.as_deref(), |tell| match convert_pdf_never_fail(&pdf_bytes, req.password.as_deref(), &params) {
        Ok((out_bytes, _, report)) => {
            // Only the chosen format lands in the chosen folder, under a name no other file has.
            let out_dir = crate::places::dir(req.place.as_deref());
            let _ = fs::create_dir_all(&out_dir);
            let clean_stem = req
                .filename
                .trim_end_matches(".pdf")
                .trim_end_matches(".PDF")
                .replace(['/', ':', '*', '?', '"', '<', '>', '|', '\\'], " ");
            let mut saved_path = crate::places::unique_path(&out_dir, clean_stem.trim(), ext);
            // Which app the new file opens in, and whether the person refused to let the app use
            // Pages (then the page says how to allow it).
            let mut opens_in = if format == OutputFormat::Docx { "word" } else { "pages" };
            let mut pages_refused = false;

            // When Pages cannot make the `.pages` (the person refused, Pages is missing or stuck),
            // the person gets the exact-layout DOCX in the chosen folder, opened in Pages when it
            // is there: the file the result screen shows is always the one they edit.
            let written = if via_pages {
                tell(serde_json::json!({ "step": "pages" }));
                let docx_path = crate::places::work_dir().join(saved_path.with_extension("docx").file_name().unwrap_or_default());
                let made_by_pages = fs::write(&docx_path, &out_bytes)
                    .map_err(|_| crate::pages_app::PagesError::Failed)
                    .and_then(|()| crate::pages_app::try_convert_docx(&docx_path, &saved_path, crate::pages_app::PagesOutput::Pages));
                let _ = fs::remove_file(&docx_path);
                match made_by_pages {
                    Ok(()) => Ok(()),
                    Err(why) => {
                        // Pages may have left the empty file it was given to write.
                        let _ = fs::remove_file(&saved_path);
                        pages_refused = matches!(why, crate::pages_app::PagesError::NotAllowed);
                        if matches!(why, crate::pages_app::PagesError::Missing) {
                            opens_in = "word";
                        }
                        saved_path = crate::places::unique_path(&out_dir, clean_stem.trim(), "docx");
                        fs::write(&saved_path, &out_bytes)
                    }
                }
            } else {
                fs::write(&saved_path, &out_bytes)
            };
            if let Err(e) = written {
                return Response::json(&ConvertResponse {
                    ok: false,
                    error: Some(write_error(&out_dir, &e)),
                    needs_password: false,
                    bad_password: false,
                    save_failed: true,
                    saved_path: None,
                    filename: None,
                    report: None,
                });
            }
            lock(saved_files()).insert(saved_path.clone());
            let out_filename = saved_path.file_name().unwrap_or_default().to_string_lossy().into_owned();

            if req.open_in_pages {
                let _ = open_file_on_mac(&saved_path, false);
            }

            let resp = ConvertResponse {
                ok: true,
                error: None,
                needs_password: false,
                bad_password: false,
                save_failed: false,
                saved_path: Some(saved_path.to_string_lossy().to_string()),
                filename: Some(out_filename),
                report: Some(report),
            };
            let mut resp = serde_json::to_value(&resp).unwrap();
            resp["place"] = place_json(&out_dir);
            resp["opens_in"] = serde_json::json!(opens_in);
            resp["pages_refused"] = serde_json::json!(pages_refused);
            Response::json(&resp)
        }
        Err(p2d_pdf::OpenError::NeedsPassword) => {
            convert_error("Ce PDF est protégé par un mot de passe.".to_string(), true, false)
        }
        Err(p2d_pdf::OpenError::BadPassword) => {
            convert_error("Le mot de passe saisi est incorrect.".to_string(), true, true)
        }
        Err(e) => convert_error(e.to_string(), false, false),
    })
}

fn open_edit_session(req: EditOpenRequest, host: &Host) -> serde_json::Value {
    let Ok(pdf_bytes) = source_pdf(req.sample_file.as_deref(), req.base64_pdf.as_deref(), host) else {
        return serde_json::json!({ "ok": false, "error": "Impossible de lire ce fichier PDF." });
    };
    let pdf = match p2d_engine::open_for_editing(&pdf_bytes, req.password.as_deref()) {
        Ok(pdf) => pdf,
        Err(p2d_pdf::OpenError::NeedsPassword | p2d_pdf::OpenError::BadPassword) => {
            return serde_json::json!({ "ok": false, "needs_password": true, "error": "Ce PDF est protégé par un mot de passe." });
        }
        Err(e) => return serde_json::json!({ "ok": false, "error": e.to_string() }),
    };
    // Saving into an encrypted PDF is not possible yet: say so before any typing, not at « Enregistrer ».
    if let Err(p2d_pdf::edit::EditError::Encrypted) = p2d_engine::save_line_edits(&pdf, &[]) {
        return serde_json::json!({
            "ok": false,
            "error": "Ce PDF est protégé contre les modifications : il ne peut pas être corrigé directement. Revenez en arrière et choisissez « Modifier dans Pages » ou « Modifier dans Word »."
        });
    }
    let id = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let session = EditSession {
        filename: req.filename,
        pdf,
        web_fonts: HashMap::new(),
        preview: None,
    };
    let view = edit_session_view(&id, &session);
    let mut sessions = lock(edit_sessions());
    // Only the last few PDFs opened stay in memory (ids are opening times).
    while sessions.len() >= KEPT_EDIT_SESSIONS {
        let Some(oldest) = sessions.keys().min_by_key(|id| u128::from_str_radix(id, 16).unwrap_or(0)).cloned() else {
            break;
        };
        sessions.remove(&oldest);
    }
    sessions.insert(id, session);
    view
}

fn edit_session_view(id: &str, session: &EditSession) -> serde_json::Value {
    let pages: Vec<EditPage> = session
        .pdf
        .doc
        .pages
        .iter()
        .zip(&session.pdf.pages)
        .enumerate()
        .map(|(i, (page, lines))| {
            let colors = session
                .pdf
                .backgrounds
                .get(i)
                .cloned()
                .unwrap_or_else(|| vec![p2d_ir::RgbaColor::WHITE; lines.len()]);
            EditPage {
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
                            .map(|run| session.pdf.run_font(i, run).map(|f| session.pdf.fonts[f].family.clone()))
                            .collect(),
                        line,
                        background,
                    })
                    .collect(),
            }
        })
        .collect();
    let fonts: Vec<EditFontView> = session
        .pdf
        .fonts
        .iter()
        .enumerate()
        .map(|(i, f)| EditFontView {
            family: f.family.clone(),
            url: format!("/api/edit/font?session={id}&font={i}"),
            bold: f.bold,
            italic: f.italic,
            size_adjust_pct: 100.0 / f.size_scale.max(0.05),
        })
        .collect();
    serde_json::json!({ "ok": true, "session": id, "filename": session.filename, "pages": pages, "fonts": fonts })
}

/// Apply pending edits in memory so the page images show exactly what saving will write.
fn preview_edit_session(req: EditPreviewRequest) -> serde_json::Value {
    let mut sessions = lock(edit_sessions());
    let Some(session) = sessions.get_mut(&req.session) else {
        return serde_json::json!({ "ok": false, "error": "Ce document n’est plus ouvert : rechargez-le." });
    };
    if req.edits.is_empty() {
        session.preview = None;
        return serde_json::json!({ "ok": true, "covered": 0 });
    }
    match p2d_engine::save_line_edits(&session.pdf, &req.edits) {
        Ok(saved) => {
            session.preview = Some(std::sync::Arc::from(saved.pdf));
            // The page fits each edited line's clickable zone to where its new text reaches.
            serde_json::json!({ "ok": true, "covered": saved.covered, "reach": saved.reach })
        }
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
    }
}

fn save_edit_session(req: EditSaveRequest) -> serde_json::Value {
    let mut sessions = lock(edit_sessions());
    let Some(session) = sessions.get_mut(&req.session) else {
        return serde_json::json!({ "ok": false, "error": "Ce document n’est plus ouvert : rechargez-le." });
    };
    let (bytes, covered) = match p2d_engine::save_line_edits(&session.pdf, &req.edits) {
        Ok(saved) => (saved.pdf, saved.covered),
        Err(e) => return serde_json::json!({ "ok": false, "error": e.to_string() }),
    };
    let out_dir = crate::places::dir(req.place.as_deref());
    let _ = fs::create_dir_all(&out_dir);
    let stem = session
        .filename
        .trim_end_matches(".pdf")
        .trim_end_matches(".PDF")
        .trim_end_matches(" (modifié)")
        .replace(['/', ':', '*', '?', '"', '<', '>', '|', '\\'], " ");
    // Saving again from the same session replaces the file it wrote; any other file is kept.
    let previous = lock(edit_saved_paths()).get(&req.session).cloned();
    let saved_path = match previous {
        Some(previous) if previous.parent() == Some(out_dir.as_path()) => previous,
        _ => crate::places::unique_path(&out_dir, &format!("{} (modifié)", stem.trim()), "pdf"),
    };
    let filename = saved_path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    if let Err(e) = fs::write(&saved_path, &bytes) {
        return serde_json::json!({ "ok": false, "error": write_error(&out_dir, &e), "save_failed": true });
    }
    lock(edit_saved_paths()).insert(req.session.clone(), saved_path.clone());
    lock(saved_files()).insert(saved_path.clone());
    // Keep editing the saved version: further edits stack on top of these.
    if let Ok(pdf) = p2d_engine::open_for_editing(&bytes, None) {
        session.pdf = pdf;
        session.filename = filename.clone();
        session.web_fonts.clear();
        session.preview = None;
    }
    let mut view = edit_session_view(&req.session, session);
    view["saved_path"] = serde_json::json!(saved_path.to_string_lossy());
    view["saved_filename"] = serde_json::json!(filename);
    view["place"] = place_json(&out_dir);
    view["covered"] = serde_json::json!(covered);
    view
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
        .map(url_decode)
}

fn write_response(stream: &mut TcpStream, resp: &Response) -> Result<(), String> {
    let reason = match resp.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        resp.status,
        resp.content_type,
        resp.body.len()
    );
    stream.write_all(header.as_bytes()).map_err(|e| e.to_string())?;
    stream.write_all(&resp.body).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn resolve_sample_pdf_path(sample_name: &str) -> PathBuf {
    let safe_name = Path::new(sample_name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("01_lettre_assurance.pdf");

    if let Some(p) = private_sample(safe_name) {
        if is_materialized_file(&p) {
            return p;
        }
    }

    Path::new("qa/corpus/synthetic").join(safe_name)
}

/// Where a `dropbox_*` example lives: real PDFs of the developer, listed outside git in
/// `qa/corpus/private/samples.json` as `{ "dropbox_facture.pdf": "~/Library/CloudStorage/…/x.pdf" }`
/// so that neither the program nor the repository names them.
fn private_sample(name: &str) -> Option<PathBuf> {
    let list: serde_json::Value = serde_json::from_slice(&fs::read("qa/corpus/private/samples.json").ok()?).ok()?;
    let path = list.get(name)?.as_str()?;
    Some(match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var("HOME").ok()?).join(rest),
        None => PathBuf::from(path),
    })
}

fn is_materialized_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.blocks() > 0 || meta.len() == 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn open_file_on_mac(path: &Path, reveal_in_finder: bool) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if reveal_in_finder {
        Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    } else if let Some(bundle) = crate::pages_app::pages_bundle().filter(|_| ext == "docx" || ext == "pages") {
        // Open in Pages rather than whatever app owns `.docx` (Word, LibreOffice…), choosing the
        // Pages that is current on this Mac (see `pages_app::pages_bundle`).
        Command::new("open")
            .args(["-b", bundle])
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    } else {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn url_decode(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let raw = s.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let (Some(hi), Some(lo)) = (
                (raw[i + 1] as char).to_digit(16),
                (raw[i + 2] as char).to_digit(16),
            ) {
                bytes.push(((hi << 4) | lo) as u8);
                i += 3;
                continue;
            }
        } else if raw[i] == b'+' {
            bytes.push(b' ');
            i += 1;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).to_string()
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for b in input.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return Err("Caractère Base64 invalide".to_string()),
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}

pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (chunk.get(1).copied().unwrap_or(0) as u32) << 8
            | chunk.get(2).copied().unwrap_or(0) as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_host() -> Host {
        Host { pages_from_disk: false, samples: false, ask_folder: Box::new(|_| None) }
    }

    fn get(path: &str) -> Response {
        respond(&Request { method: "GET".into(), path: path.into(), query: String::new(), body: Vec::new() }, &dev_host())
    }

    fn post(path: &str, body: &str) -> Response {
        respond(&Request { method: "POST".into(), path: path.into(), query: String::new(), body: body.into() }, &dev_host())
    }

    #[test]
    fn base64_round_trips() {
        for len in 0..10 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 200) as u8).collect();
            assert_eq!(decode_base64(&encode_base64(&bytes)).unwrap(), bytes);
        }
        assert_eq!(encode_base64(b"Bonjour"), "Qm9uam91cg==");
    }

    #[test]
    fn only_own_pages_reach_the_local_server() {
        assert!(from_own_page(7878, Some("127.0.0.1:7878"), None, None));
        assert!(from_own_page(7878, Some("localhost:7878"), Some("http://localhost:7878"), Some("same-origin")));
        assert!(from_own_page(7878, Some("127.0.0.1:7878"), None, Some("none")));
        // Another web site, another local server, DNS rebinding, no Host at all.
        assert!(!from_own_page(7878, Some("127.0.0.1:7878"), Some("https://evil.example"), Some("cross-site")));
        assert!(!from_own_page(7878, Some("127.0.0.1:7878"), Some("http://127.0.0.1:3000"), Some("same-site")));
        assert!(!from_own_page(7878, Some("127.0.0.1:7878"), Some("null"), None));
        assert!(!from_own_page(7878, Some("evil.example:7878"), None, None));
        assert!(!from_own_page(7878, None, None, None));
    }

    #[test]
    fn serves_the_built_in_pages_and_nothing_else() {
        let home = get("/");
        assert_eq!((home.status, home.content_type), (200, "text/html; charset=utf-8"));
        assert_eq!(get("/edit.js").status, 200);
        assert_eq!(get("/../Cargo.toml").status, 404);
        assert_eq!(get("/%2e%2e/Cargo.toml").status, 404);
        assert_eq!(get("//etc/hosts").status, 404);
        assert_eq!(get("/api/samples").status, 404);
    }

    #[test]
    fn tells_the_waiting_page_which_step_is_running() {
        let ask = |job: &str| -> serde_json::Value {
            let req = Request { method: "GET".into(), path: "/api/progress".into(), query: format!("job={job}"), body: Vec::new() };
            serde_json::from_slice(&respond(&req, &dev_host()).body).unwrap()
        };
        let (engine, server) = with_progress(Some("t-1"), |tell| {
            p2d_ir::progress::report(p2d_ir::progress::Step::Read { page: 2, pages: 3 });
            let engine = ask("t-1");
            tell(serde_json::json!({ "step": "pages" }));
            (engine, ask("t-1"))
        });
        assert_eq!(engine["progress"], serde_json::json!({ "step": "read", "page": 2, "pages": 3 }));
        assert_eq!(server["progress"]["step"], "pages");
        assert!(ask("t-1")["progress"].is_null(), "forgotten once done");
    }

    #[test]
    fn opens_only_files_the_app_saved() {
        let resp = post("/api/action", r#"{"saved_path":"/etc/hosts","action":"open"}"#);
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn examples_are_off_in_the_mac_app() {
        let resp = post("/api/convert", r#"{"filename":"x.pdf","sample_file":"01_lettre_assurance.pdf","format":"docx"}"#);
        let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(json["ok"], false);
        assert!(json["error"].as_str().unwrap().contains("exemples"));
    }

    #[test]
    fn hands_an_opened_pdf_to_the_page_once() {
        queue_opened_pdf("Lettre.pdf".into(), b"%PDF-1.4".to_vec());
        let peek = Request { method: "GET".into(), path: "/api/opened-pdf".into(), query: "peek=1".into(), body: Vec::new() };
        let peeked: serde_json::Value = serde_json::from_slice(&respond(&peek, &dev_host()).body).unwrap();
        assert_eq!(peeked["waiting"], true);
        let first: serde_json::Value = serde_json::from_slice(&get("/api/opened-pdf").body).unwrap();
        assert_eq!((first["ok"].as_bool(), first["name"].as_str()), (Some(true), Some("Lettre.pdf")));
        assert_eq!(decode_base64(first["base64"].as_str().unwrap()).unwrap(), b"%PDF-1.4");
        let second: serde_json::Value = serde_json::from_slice(&get("/api/opened-pdf").body).unwrap();
        assert_eq!(second["ok"], false);
    }
}
