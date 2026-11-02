//! The Mac app: one window showing the app's pages, served from inside the program through the
//! private `app://` address (no network port, so no web page or other program can call the API),
//! with French menus, ⌘O, the text size, and PDFs sent by the Finder (« Ouvrir avec », dropped on
//! the Dock icon). `--self-test` loads the pages in a hidden window and checks the API end to end,
//! for the packaging script.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tao::window::{Window, WindowBuilder};
use wry::http::{Response as HttpResponse, header};
use wry::{NewWindowResponse, PageLoadEvent, WebView, WebViewBuilder, WebViewBuilderExtDarwin};

use crate::serve::{self, Host, Request};

const APP_NAME: &str = "PDF Edit";
const HOME_URL: &str = "app://localhost/drop.html";
/// The pages' background (drop.css `--bg`), shown while a page loads instead of a white flash.
const BACKGROUND: (u8, u8, u8, u8) = (250, 249, 246, 255);
/// Text sizes offered by ⌘+ and ⌘-.
const ZOOM_STEPS: [f64; 8] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
/// With corrections not saved, closing asks the editor page to show its « Vos corrections ne sont
/// pas enregistrées » question (edit.js); `false` when the page cannot, and the app then quits.
const CONFIRM_QUIT_SCRIPT: &str = "(() => { try { if (typeof window.modifierMesPdfConfirmQuit === 'function') { window.modifierMesPdfConfirmQuit(); return true; } } catch (e) {} return false; })()";

enum UserEvent {
    Menu(MenuEvent),
    OpenPdf(PathBuf),
    /// The red button, ⌘W, ⌘Q, or « Quitter » from the Dock, the app switcher, logging out.
    CloseRequested,
    /// Quit now: nothing is left unsaved, or the person chose « Quitter sans enregistrer ».
    Quit,
    /// The page's process stopped (crash, memory pressure): the window would stay blank.
    PageGone,
    SelfTestReport(String),
}

type Proxy = Arc<Mutex<EventLoopProxy<UserEvent>>>;

fn send(proxy: &Proxy, event: UserEvent) {
    let _ = proxy.lock().unwrap_or_else(PoisonError::into_inner).send_event(event);
}

struct QuitHandlerIvars {
    proxy: Proxy,
}

define_class!(
    /// « Quitter » from the Dock or the app switcher, logging out and shutting down reach the app as
    /// the "quit" Apple event, which AppKit would obey at once: this handler sends it through the
    /// same check for unsaved corrections as ⌘Q.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = QuitHandlerIvars]
    struct QuitHandler;

    unsafe impl NSObjectProtocol for QuitHandler {}

    impl QuitHandler {
        #[unsafe(method(handleQuitEvent:withReplyEvent:))]
        fn handle_quit_event(&self, _event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            send(&self.ivars().proxy, UserEvent::CloseRequested);
        }
    }
);

impl QuitHandler {
    /// Replaces AppKit's own handler, so call it once the app has finished launching.
    fn install(mtm: MainThreadMarker, proxy: Proxy) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(QuitHandlerIvars { proxy });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let manager = NSAppleEventManager::sharedAppleEventManager();
        // kCoreEventClass 'aevt', kAEQuitApplication 'quit'.
        let () = unsafe {
            msg_send![&*manager, setEventHandler: &*this, andSelector: sel!(handleQuitEvent:withReplyEvent:), forEventClass: u32::from_be_bytes(*b"aevt"), andEventID: u32::from_be_bytes(*b"quit")]
        };
        this
    }
}

struct AppMenu {
    bar: Menu,
    window_menu: Submenu,
    open: MenuItem,
    close: MenuItem,
    quit: MenuItem,
    zoom_in: MenuItem,
    zoom_out: MenuItem,
    zoom_reset: MenuItem,
}

impl AppMenu {
    fn new() -> Self {
        let cmd = |code| Some(Accelerator::new(Modifiers::META, code));
        let open = MenuItem::new("Ouvrir un PDF…", true, cmd(Code::KeyO));
        let close = MenuItem::new("Fermer la fenêtre", true, cmd(Code::KeyW));
        let quit = MenuItem::new(format!("Quitter {APP_NAME}"), true, cmd(Code::KeyQ));
        let zoom_in = MenuItem::new("Agrandir le texte", true, cmd(Code::Equal));
        let zoom_out = MenuItem::new("Réduire le texte", true, cmd(Code::Minus));
        let zoom_reset = MenuItem::new("Taille normale du texte", true, cmd(Code::Digit0));
        let about = AboutMetadata {
            name: Some(APP_NAME.to_string()),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            copyright: Some("© 2026 Makio".to_string()),
            ..Default::default()
        };
        let separator = PredefinedMenuItem::separator;

        let app_menu = Submenu::with_items(
            APP_NAME,
            true,
            &[
                &PredefinedMenuItem::about(Some(&format!("À propos de {APP_NAME}")), Some(about)),
                &separator(),
                &PredefinedMenuItem::services(Some("Services")),
                &separator(),
                &PredefinedMenuItem::hide(Some(&format!("Masquer {APP_NAME}"))),
                &PredefinedMenuItem::hide_others(Some("Masquer les autres")),
                &PredefinedMenuItem::show_all(Some("Tout afficher")),
                &separator(),
                &quit,
            ],
        )
        .expect("app menu");
        let file_menu = Submenu::with_items("Fichier", true, &[&open, &separator(), &close]).expect("file menu");
        let edit_menu = Submenu::with_items(
            "Édition",
            true,
            &[
                &PredefinedMenuItem::undo(Some("Annuler")),
                &PredefinedMenuItem::redo(Some("Rétablir")),
                &separator(),
                &PredefinedMenuItem::cut(Some("Couper")),
                &PredefinedMenuItem::copy(Some("Copier")),
                &PredefinedMenuItem::paste(Some("Coller")),
                &PredefinedMenuItem::select_all(Some("Tout sélectionner")),
            ],
        )
        .expect("edit menu");
        let view_menu = Submenu::with_items(
            "Présentation",
            true,
            &[
                &zoom_in,
                &zoom_out,
                &zoom_reset,
                &separator(),
                &PredefinedMenuItem::fullscreen(Some("Plein écran")),
            ],
        )
        .expect("view menu");
        let window_menu = Submenu::with_items(
            "Fenêtre",
            true,
            &[
                &PredefinedMenuItem::minimize(Some("Placer dans le Dock")),
                &PredefinedMenuItem::maximize(Some("Zoom")),
                &separator(),
                &PredefinedMenuItem::bring_all_to_front(Some("Tout ramener au premier plan")),
            ],
        )
        .expect("window menu");
        let bar = Menu::with_items(&[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu]).expect("menu bar");
        Self { bar, window_menu, open, close, quit, zoom_in, zoom_out, zoom_reset }
    }
}

/// Open the window and run the app until it quits. With `self_test`, the window stays hidden, the
/// pages check the API and the process exits with the result (0: everything works).
pub fn run(self_test: bool) {
    if !self_test {
        log_panics();
    }
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    if self_test {
        // Neither a Dock icon nor a menu bar for the check.
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy: Proxy = Arc::new(Mutex::new(event_loop.create_proxy()));
    {
        let proxy = Arc::clone(&proxy);
        MenuEvent::set_event_handler(Some(move |event| send(&proxy, UserEvent::Menu(event))));
    }
    let menu = AppMenu::new();

    let window = WindowBuilder::new()
        .with_title(APP_NAME)
        .with_inner_size(LogicalSize::new(1100.0, 820.0))
        .with_min_inner_size(LogicalSize::new(700.0, 520.0))
        .with_visible(false)
        .build(&event_loop)
        .expect("window");
    fit_and_center(&window);

    let host = Arc::new(Host {
        pages_from_disk: false,
        samples: false,
        ask_folder: Box::new(ask_folder),
    });
    // Whether the editor holds corrections not saved yet, as it reports through `window.ipc`
    // (edit.js); every new page starts with none.
    let unsaved = Arc::new(AtomicBool::new(false));
    let builder = WebViewBuilder::new()
        .with_url(HOME_URL)
        .with_background_color(BACKGROUND)
        .with_initialization_script(format!(
            "window.modifierMesPdfApp = {{ version: {:?} }};",
            env!("CARGO_PKG_VERSION")
        ))
        .with_asynchronous_custom_protocol("app".into(), move |_webview, request, responder| {
            let host = Arc::clone(&host);
            // Conversions take seconds and the folder dialog waits for the person: answer off the
            // main thread.
            std::thread::spawn(move || {
                let (parts, body) = request.into_parts();
                let req = Request {
                    method: parts.method.to_string(),
                    path: parts.uri.path().to_string(),
                    query: parts.uri.query().unwrap_or("").to_string(),
                    body,
                };
                let resp = serve::respond(&req, &host);
                let response = HttpResponse::builder()
                    .status(resp.status)
                    .header(header::CONTENT_TYPE, resp.content_type)
                    .header(header::CACHE_CONTROL, "no-store")
                    .body(resp.body)
                    .expect("response");
                responder.respond(response);
            });
        })
        // Only the app's pages show in the window; a web link opens in the browser.
        .with_navigation_handler(|url| {
            if url.starts_with("app://") || url.starts_with("about:") {
                return true;
            }
            open_outside(&url);
            false
        })
        .with_new_window_req_handler(|url, _| {
            open_outside(&url);
            NewWindowResponse::Deny
        })
        .with_download_started_handler(|_, _| false)
        .with_accept_first_mouse(true)
        .with_devtools(cfg!(debug_assertions))
        .with_on_page_load_handler({
            let unsaved = Arc::clone(&unsaved);
            move |event, _| {
                if let PageLoadEvent::Started = event {
                    unsaved.store(false, Ordering::SeqCst);
                }
            }
        })
        .with_ipc_handler({
            let (unsaved, proxy) = (Arc::clone(&unsaved), Arc::clone(&proxy));
            move |request| {
                let message: serde_json::Value = serde_json::from_str(request.body()).unwrap_or_default();
                if let Some(state) = message["unsaved"].as_bool() {
                    unsaved.store(state, Ordering::SeqCst);
                }
                if message["quit"] == true {
                    send(&proxy, UserEvent::Quit);
                }
                if self_test && message.get("selfTest").is_some() {
                    send(&proxy, UserEvent::SelfTestReport(message["selfTest"].to_string()));
                }
            }
        })
        .with_on_web_content_process_terminate_handler({
            let proxy = Arc::clone(&proxy);
            move || send(&proxy, UserEvent::PageGone)
        });
    let builder = if self_test { builder.with_initialization_script(self_test_script()) } else { builder };
    let webview = builder.build(&window).expect("webview");
    let mut zoom = crate::settings::get("zoom")
        .and_then(|z| z.as_f64())
        .filter(|z| ZOOM_STEPS.contains(z))
        .unwrap_or(1.0);
    let _ = webview.zoom(zoom);
    if self_test {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(60));
            println!("{{\"ok\":false,\"error\":\"no answer from the page within 60 s\"}}");
            std::process::exit(2);
        });
    } else {
        window.set_visible(true);
    }

    let mut quit_handler = None;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        // Quit, unless the editor holds corrections not saved: then the page asks the person, and
        // sends « quit » if they choose to (UserEvent::Quit).
        let close = |control_flow: &mut ControlFlow| {
            if !unsaved.load(Ordering::SeqCst) {
                *control_flow = ControlFlow::Exit;
                return;
            }
            window.set_minimized(false);
            window.set_focus();
            let proxy = Arc::clone(&proxy);
            let asked = webview.evaluate_script_with_callback(CONFIRM_QUIT_SCRIPT, move |shown| {
                if shown != "true" {
                    send(&proxy, UserEvent::Quit);
                }
            });
            if asked.is_err() {
                *control_flow = ControlFlow::Exit;
            }
        };
        match event {
            Event::NewEvents(StartCause::Init) if !self_test => {
                menu.bar.init_for_nsapp();
                menu.window_menu.set_as_windows_menu_for_nsapp();
                if let Some(mtm) = MainThreadMarker::new() {
                    quit_handler = Some(QuitHandler::install(mtm, Arc::clone(&proxy)));
                }
            }
            Event::Opened { urls } => {
                let pdf = urls.iter().filter_map(|url| url.to_file_path().ok()).find(|path| is_pdf(path));
                if let Some(path) = pdf {
                    open_pdf(&window, &webview, &path);
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UserEvent::CloseRequested) => {
                close(control_flow)
            }
            Event::UserEvent(UserEvent::Quit) => *control_flow = ControlFlow::Exit,
            Event::UserEvent(UserEvent::PageGone) => {
                unsaved.store(false, Ordering::SeqCst);
                let _ = webview.load_url(HOME_URL);
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                let id = &event.id;
                if id == menu.open.id() {
                    choose_pdf(Arc::clone(&proxy));
                } else if id == menu.close.id() || id == menu.quit.id() {
                    close(control_flow);
                } else if id == menu.zoom_in.id() || id == menu.zoom_out.id() || id == menu.zoom_reset.id() {
                    let step = ZOOM_STEPS.iter().position(|z| *z == zoom).unwrap_or(2);
                    zoom = if id == menu.zoom_in.id() {
                        ZOOM_STEPS[(step + 1).min(ZOOM_STEPS.len() - 1)]
                    } else if id == menu.zoom_out.id() {
                        ZOOM_STEPS[step.saturating_sub(1)]
                    } else {
                        1.0
                    };
                    let _ = webview.zoom(zoom);
                    crate::settings::set("zoom", serde_json::json!(zoom));
                }
            }
            Event::UserEvent(UserEvent::OpenPdf(path)) => open_pdf(&window, &webview, &path),
            Event::UserEvent(UserEvent::SelfTestReport(report)) => {
                println!("{report}");
                let ok = serde_json::from_str::<serde_json::Value>(&report).is_ok_and(|r| r["ok"] == true);
                std::process::exit(if ok { 0 } else { 1 });
            }
            _ => {}
        }
        // Kept alive for the app's lifetime: AppKit does not retain Apple event handlers.
        let _ = &quit_handler;
    });
}

/// The window at its usual size, smaller on a small screen, in the middle of the screen.
fn fit_and_center(window: &Window) {
    let Some(monitor) = window.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let screen: LogicalSize<f64> = monitor.size().to_logical(scale);
    let origin: LogicalPosition<f64> = monitor.position().to_logical(scale);
    let size = LogicalSize::new(1100f64.min(screen.width * 0.92), 820f64.min(screen.height * 0.85));
    window.set_inner_size(size);
    window.set_outer_position(LogicalPosition::new(
        origin.x + (screen.width - size.width) / 2.0,
        origin.y + (screen.height - size.height) / 2.0 * 0.8,
    ));
}

fn is_pdf(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

/// Give a PDF from the Finder or ⌘O to the page: the home page takes it to step 2, the editor goes
/// back to the home page first (see drop.js and edit.js).
fn open_pdf(window: &Window, webview: &WebView, path: &Path) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let name = path.file_name().map_or_else(|| "document.pdf".to_string(), |n| n.to_string_lossy().into_owned());
    serve::queue_opened_pdf(name, bytes);
    window.set_minimized(false);
    window.set_focus();
    let _ = webview.evaluate_script("window.dispatchEvent(new Event('modifier-mes-pdf:opened'))");
}

/// ⌘O: the Mac's file dialog. Shown from another thread: the dialog then runs on the main thread by
/// itself (rfd), outside the window's event handler.
fn choose_pdf(proxy: Proxy) {
    std::thread::spawn(move || {
        let documents = crate::settings::home().join("Documents");
        let chosen = rfd::FileDialog::new()
            .set_title("Choisissez le PDF à modifier")
            .add_filter("PDF", &["pdf", "PDF"])
            .set_directory(documents)
            .pick_file();
        if let Some(path) = chosen {
            send(&proxy, UserEvent::OpenPdf(path));
        }
    });
}

/// « Choisir un dossier… »: called by the page's request thread, which waits for the answer.
fn ask_folder(start: &Path) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(crate::places::FOLDER_PROMPT)
        .set_directory(start)
        .set_can_create_directories(true)
        .pick_folder()
}

/// A web link from a page: in the person's browser (or mail app), never in the app's window.
fn open_outside(url: &str) {
    if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}

/// Write crashes to ~/Library/Logs/Modifier mes PDF/, where they can be found to fix them.
fn log_panics() {
    let dir = crate::settings::home().join("Library/Logs/Modifier mes PDF");
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(dir.join("Modifier mes PDF.log")) {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let _ = writeln!(log, "[{now}] version {} : {info}", env!("CARGO_PKG_VERSION"));
        }
        default_hook(info);
    }));
}

/// The check run by `--self-test` in the home page, reported through `window.ipc`.
fn self_test_script() -> String {
    let (pdf, _) = crate::corpus::build_letter_pdf(
        "Madame, Monsieur,",
        &["Votre dossier est complet.", "Veuillez agréer nos salutations distinguées."],
    );
    format!("window.SELF_TEST_PDF = {:?};\n{}", serve::encode_base64(&pdf), SELF_TEST_JS)
}

const SELF_TEST_JS: &str = r#"
(() => {
  if (location.pathname !== '/drop.html') return;
  addEventListener('load', async () => {
    const checks = {};
    // Why a check failed, when the API said.
    const details = {};
    const post = (url, body) =>
      fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
    try {
      checks.page = document.title === 'PDF Edit' && !document.getElementById('stepFile').hidden;
      checks.styles = getComputedStyle(document.body).fontSize !== '16px';
      checks.samplesHidden = document.getElementById('samplesBox').hidden === true;
      checks.editorPage = (await fetch('/edit.html')).ok && (await fetch('/edit.js')).ok;
      checks.places = (await (await fetch('/api/save-places')).json()).ok === true;
      // A large body, as with a big PDF: the whole request must reach the app.
      const open = await (
        await post('/api/edit/open', { filename: 'self-test.pdf', base64_pdf: window.SELF_TEST_PDF, padding: 'x'.repeat(24 << 20) })
      ).json();
      details.open = open.error;
      checks.open = open.ok === true && open.pages.length === 2 && open.pages[0].lines.length > 0;
      const png = new Uint8Array(await (await fetch(`/api/edit/page.png?session=${open.session}&page=0`)).arrayBuffer());
      checks.render = png.length > 1000 && png[1] === 0x50 && png[2] === 0x4e && png[3] === 0x47;
      const line = open.pages[0].lines[0];
      const preview = await (
        await post('/api/edit/preview', { session: open.session, edits: [{ page_index: 0, line_id: line.id, text: 'Bonjour Madame' }] })
      ).json();
      details.preview = preview.error;
      const edited = new Uint8Array(
        await (await fetch(`/api/edit/page.png?session=${open.session}&page=0&preview=1`)).arrayBuffer()
      );
      checks.preview = preview.ok === true && edited.length > 1000 && edited.some((byte, i) => byte !== png[i]);
      checks.foreignFileRefused = (await post('/api/action', { saved_path: '/etc/hosts', action: 'open' })).status === 404;
    } catch (err) {
      checks.error = String((err && err.message) || err);
    }
    const ok = Object.values(checks).every((v) => v === true);
    window.ipc.postMessage(
      JSON.stringify({ selfTest: { ok, checks, details, version: window.modifierMesPdfApp && window.modifierMesPdfApp.version } })
    );
  });
})();
"#;
