//! What the app remembers on this Mac between launches (the folder picked last time, the text
//! size), in ~/Library/Application Support/Modifier mes PDF/settings.json.
//! Its folders under ~/Library (settings, caches, logs) keep the app's first name, so renaming it
//! « PDF Edit » left everything where it was.

use std::fs;
use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

fn path() -> PathBuf {
    home().join("Library/Application Support/Modifier mes PDF/settings.json")
}

fn read() -> serde_json::Map<String, serde_json::Value> {
    fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn get(key: &str) -> Option<serde_json::Value> {
    read().remove(key)
}

/// Change one setting, keeping the others.
pub fn set(key: &str, value: serde_json::Value) {
    let mut settings = read();
    settings.insert(key.to_string(), value);
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, serde_json::Value::Object(settings).to_string());
}
