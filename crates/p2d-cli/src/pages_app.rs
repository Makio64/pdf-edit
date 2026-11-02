//! Driving Apple Pages through AppleScript: export a DOCX to PDF, or save it as a real `.pages`.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

/// The current Pages ("Pages Creator Studio", 15+) and the classic one (14.x). When both are
/// installed, macOS treats the classic app as out of date: launching it only shows "Pages 14.5 is
/// out of date and can be deleted" and it refuses automation, so it is never used as a fallback.
const CURRENT_PAGES: &str = "com.apple.Pages";
const CLASSIC_PAGES: &str = "com.apple.iWork.Pages";

/// Bundle identifier of the Pages app to use on this Mac, or `None` when Pages is not installed.
pub fn pages_bundle() -> Option<&'static str> {
    static BUNDLE: OnceLock<Option<&'static str>> = OnceLock::new();
    *BUNDLE.get_or_init(|| [CURRENT_PAGES, CLASSIC_PAGES].into_iter().find(|id| is_installed(id)))
}

/// Whether LaunchServices knows an app with this bundle identifier (without launching it).
fn is_installed(bundle_id: &str) -> bool {
    let script = format!(
        r#"ObjC.import("AppKit"); $.NSWorkspace.sharedWorkspace.URLForApplicationWithBundleIdentifier("{bundle_id}").isNil() ? "no" : "yes""#
    );
    Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout.starts_with(b"yes"))
}

pub enum PagesOutput {
    Pdf,
    Pages,
}

/// Why Pages did not write the file.
pub enum PagesError {
    /// Pages is not installed on this Mac.
    Missing,
    /// The person refused to let the app control Pages (Réglages Système › Automatisation).
    NotAllowed,
    Failed,
}

/// Open `docx` in Pages and write it to `target` as `output`. Returns whether it worked.
pub fn convert_docx(docx: &Path, target: &Path, output: PagesOutput) -> bool {
    try_convert_docx(docx, target, output).is_ok()
}

/// `convert_docx`, saying why it failed. The target file is created first so that Pages' sandbox
/// may write it: on failure it may be left empty.
pub fn try_convert_docx(docx: &Path, target: &Path, output: PagesOutput) -> Result<(), PagesError> {
    let Some(bundle) = pages_bundle() else {
        return Err(PagesError::Missing);
    };
    let (Ok(docx), Ok(())) = (fs::canonicalize(docx), fs::write(target, b"")) else {
        return Err(PagesError::Failed);
    };
    let Ok(target) = fs::canonicalize(target) else {
        return Err(PagesError::Failed);
    };
    let command = match output {
        PagesOutput::Pdf => format!(r#"export d to (POSIX file "{}") as PDF"#, target.display()),
        PagesOutput::Pages => format!(r#"save d in (POSIX file "{}")"#, target.display()),
    };
    let script = format!(
        r#"with timeout of 120 seconds
            tell application id "{bundle}"
                set d to open (POSIX file "{}")
                {command}
                close d saving no
            end tell
        end timeout"#,
        docx.display()
    );
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let _ = fs::write(&target, b"");
        }
        let run = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|_| PagesError::Failed)?;
        if !run.status.success() {
            // -1743 (errAEEventNotPermitted) answers « Ne pas autoriser »; -1744 when macOS could not
            // even ask.
            let stderr = String::from_utf8_lossy(&run.stderr);
            let refused = stderr.contains("-1743") || stderr.contains("-1744") || stderr.contains("Not authorized");
            if refused {
                return Err(PagesError::NotAllowed);
            }
            continue;
        }
        if fs::metadata(&target).is_ok_and(|m| m.len() > 100) {
            return Ok(());
        }
    }
    Err(PagesError::Failed)
}
