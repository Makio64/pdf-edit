//! The app's pages (`extension/ui`), built into the program: the Mac app needs no other file.

const FILES: [(&str, &[u8]); 6] = [
    ("drop.html", include_bytes!("../../../extension/ui/drop.html")),
    ("drop.css", include_bytes!("../../../extension/ui/drop.css")),
    ("drop.js", include_bytes!("../../../extension/ui/drop.js")),
    ("edit.html", include_bytes!("../../../extension/ui/edit.html")),
    ("edit.css", include_bytes!("../../../extension/ui/edit.css")),
    ("edit.js", include_bytes!("../../../extension/ui/edit.js")),
];

/// A page file by its name in `extension/ui` ("drop.html").
pub fn get(name: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(file, _)| *file == name).map(|(_, bytes)| *bytes)
}

pub fn mime(name: &str) -> &'static str {
    match name.rsplit_once('.').map_or("", |(_, ext)| ext) {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "json" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every file a built-in page links to (`href="…"`, `src="…"`) is built in too.
    #[test]
    fn pages_only_link_to_built_in_files() {
        for (name, bytes) in FILES.iter().filter(|(name, _)| name.ends_with(".html")) {
            let html = std::str::from_utf8(bytes).unwrap();
            for attr in ["href=\"", "src=\""] {
                for rest in html.split(attr).skip(1) {
                    let target = rest.split('"').next().unwrap();
                    let file = target.split(['#', '?']).next().unwrap();
                    if file.is_empty() || file.contains(':') {
                        continue;
                    }
                    assert!(get(file).is_some(), "{name} links to {target}, which is not built in");
                }
            }
        }
    }
}
