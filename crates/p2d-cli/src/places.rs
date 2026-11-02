//! Where the app saves new files: on the Desktop, in Downloads, or in a folder the person picked in
//! the Mac's own « Choose a folder » dialog, which is remembered for next time.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::settings::home;

/// The folder for a place sent by the page: "desktop", "downloads" or "chosen". The chosen folder
/// is the one remembered on this Mac, never a path from the request, since any web page can call
/// the local server.
pub fn dir(place: Option<&str>) -> PathBuf {
    match place {
        Some("desktop") => home().join("Desktop"),
        Some("chosen") => last_folder().unwrap_or_else(|| home().join("Downloads")),
        _ => home().join("Downloads"),
    }
}

/// The folder picked last time, while it still exists.
pub fn last_folder() -> Option<PathBuf> {
    let folder = PathBuf::from(crate::settings::get("last_folder")?.as_str()?);
    folder.is_dir().then_some(folder)
}

/// Ask for a folder with `ask` (the Mac's own dialog), starting from the last one, and remember
/// the answer. `None` when cancelled.
pub fn choose_folder(ask: &dyn Fn(&Path) -> Option<PathBuf>) -> Option<PathBuf> {
    let start = last_folder().unwrap_or_else(|| home().join("Documents"));
    let folder = ask(&start)?;
    if !folder.is_dir() {
        return None;
    }
    crate::settings::set("last_folder", serde_json::json!(folder));
    Some(folder)
}

/// The folder dialog of the local web server (`p2d-cli app`), shown by `osascript` in front of the
/// browser. The Mac app shows its own (see `app_window`).
pub fn ask_folder_with_osascript(start: &Path) -> Option<PathBuf> {
    let start = start.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    let output = Command::new("osascript")
        // Bring the dialog in front of the browser.
        .args(["-e", "activate"])
        .args([
            "-e",
            &format!(
                "set f to choose folder with prompt \"{FOLDER_PROMPT}\" default location (POSIX file \"{start}\")"
            ),
        ])
        .args(["-e", "POSIX path of f"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let chosen = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(PathBuf::from(if chosen.len() > 1 { chosen.trim_end_matches('/') } else { &chosen }))
}

pub const FOLDER_PROMPT: &str = "Choisissez le dossier où enregistrer le nouveau fichier :";

/// How the page names a folder, one part per level: ["Bureau"], ["Documents", "Factures"],
/// ["iCloud Drive", "Courrier"], ["Clé USB", "Scans"]…
pub fn label(folder: &Path) -> Vec<String> {
    let names = |path: &Path| -> Vec<String> {
        path.components()
            .filter_map(|c| match c {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    };
    let home = home();
    let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    if let Ok(rest) = folder.strip_prefix(&icloud) {
        return [vec!["iCloud Drive".to_string()], names(rest)].concat();
    }
    let mut parts = match folder.strip_prefix(&home) {
        // The home folder itself: its name.
        Ok(rest) if rest.as_os_str().is_empty() => names(folder).into_iter().last().into_iter().collect(),
        Ok(rest) => names(rest),
        Err(_) => names(folder.strip_prefix("/Volumes").unwrap_or(folder)),
    };
    if folder.starts_with(&home) {
        if let Some(first) = parts.first_mut() {
            let french = match first.as_str() {
                "Desktop" => "Bureau",
                "Downloads" => "Téléchargements",
                "Pictures" => "Images",
                "Movies" => "Vidéos",
                "Music" => "Musique",
                other => other,
            };
            *first = french.to_string();
        }
    }
    parts
}

/// `stem.ext` in `folder`, or « stem 2.ext », « stem 3.ext »… when that name is taken: an
/// earlier file, maybe already edited, is never overwritten.
pub fn unique_path(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = folder.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| folder.join(format!("{stem} {n}.{ext}")))
        .find(|path| !path.exists())
        .unwrap_or(first)
}

/// Out-of-sight folder for intermediate files, such as the DOCX that Pages turns into `.pages`.
pub fn work_dir() -> PathBuf {
    let dir = home().join("Library/Caches/Modifier mes PDF");
    let _ = fs::create_dir_all(&dir);
    dir
}
