//! Target fonts for `Preset::Exact`: which installed Mac font reproduces a PDF font, and how wide
//! text set in it is (so word positions can be matched with character spacing).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use p2d_ir::{FontIr, Profile};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gpos::{Gpos, PairPos, PositionSubtables};
use skrifa::raw::tables::kern::{Kern, SubtableKind};
use skrifa::{FontRef, GlyphId, GlyphId16, MetadataProvider, Tag};

use crate::map_pdf_font_to_mac;

/// Measures text set in installed fonts. The native build reads the system font files; the WASM
/// build uses [`NoFontMetrics`], which reads fonts registered in memory via [`register_memory_font`].
pub trait FontMetrics {
    /// Whether a font family with this name is installed.
    fn has_family(&self, family: &str) -> bool;
    /// Distance in points from each character of `text` to the next one when set in `family` at
    /// `size_pt`: its advance plus the pair kerning Pages applies, or `None` when the family is
    /// not installed.
    fn char_steps(&self, family: &str, bold: bool, italic: bool, size_pt: f32, text: &str) -> Option<Vec<f32>>;
    /// Advance width in points of `text`, kerning included.
    fn text_width(&self, family: &str, bold: bool, italic: bool, size_pt: f32, text: &str) -> Option<f32> {
        self.char_steps(family, bold, italic, size_pt, text)
            .map(|steps| steps.iter().sum())
    }
    /// The installed family whose glyphs look most like `font`'s (see [`crate::lookalike`]), for a
    /// font the Mac does not have; `None` when that cannot be told.
    fn closest_family(&self, _font: &FontIr) -> Option<String> {
        None
    }
}

#[derive(Clone)]
struct MemoryFace {
    data: Arc<Vec<u8>>,
    bold: bool,
    italic: bool,
}

fn memory_registry() -> &'static Mutex<HashMap<String, Vec<MemoryFace>>> {
    static REG: OnceLock<Mutex<HashMap<String, Vec<MemoryFace>>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers a TrueType/OpenType font in memory (used by the online WASM build after fetching
/// missing families from Google Fonts / Fontsource CDN).
pub fn register_memory_font(family: &str, bold: bool, italic: bool, raw_ttf: &[u8]) {
    if FontRef::from_index(raw_ttf, 0).is_err() {
        return;
    }
    let key = family.trim().to_ascii_lowercase();
    let data = Arc::new(raw_ttf.to_vec());
    if let Ok(mut map) = memory_registry().lock() {
        let list = map.entry(key).or_default();
        list.retain(|f| !(f.bold == bold && f.italic == italic));
        list.push(MemoryFace { data, bold, italic });
    }
}

/// Returns the in-memory font file closest to `(family, bold, italic)`, if registered.
pub fn memory_face_file(family: &str, bold: bool, italic: bool) -> Option<(Arc<Vec<u8>>, u32)> {
    let key = family.trim().to_ascii_lowercase();
    let guard = memory_registry().lock().ok()?;
    let list = guard.get(&key)?;
    let best = list.iter().min_by_key(|f| {
        let mut s = 0u32;
        if f.bold != bold {
            s += 10;
        }
        if f.italic != italic {
            s += 20;
        }
        s
    })?;
    Some((best.data.clone(), 0))
}

pub(crate) fn char_steps_from_bytes(data: &[u8], index: u32, size_pt: f32, text: &str) -> Option<Vec<f32>> {
    let font = FontRef::from_index(data, index).ok()?;
    let charmap = font.charmap();
    let metrics = font.glyph_metrics(Size::new(size_pt), LocationRef::default());
    let fallback_gid = charmap.map('?');
    let gids: Vec<Option<GlyphId>> = text
        .chars()
        .map(|ch| {
            let ch = if ch == '\u{00A0}' || ch == '\u{202F}' { ' ' } else { ch };
            charmap.map(ch).or(fallback_gid)
        })
        .collect();
    let kern = font.kern().ok();
    let gpos = font.gpos().ok();
    let gpos_lookups = gpos.as_ref().map(collect_gpos_kern_lookups).unwrap_or_default();
    let units_per_em = font.head().map(|h| h.units_per_em()).unwrap_or(1000).max(1) as f32;
    let kern_scale = size_pt / units_per_em;
    Some(
        gids.iter()
            .enumerate()
            .map(|(i, gid)| {
                let advance = gid.and_then(|g| metrics.advance_width(g)).unwrap_or(size_pt * 0.5);
                let kerning = match (gid, gids.get(i + 1).copied().flatten()) {
                    (Some(l), Some(r)) => {
                        let legacy = kern.as_ref().map_or(0, |k| pair_kerning_shared(k, *l, r));
                        let k_units = if legacy != 0 {
                            legacy
                        } else if let Some(g) = &gpos {
                            gpos_pair_kerning(g, &gpos_lookups, *l, r)
                        } else {
                            0
                        };
                        k_units as f32 * kern_scale
                    }
                    _ => 0.0,
                };
                advance + kerning
            })
            .collect(),
    )
}

fn pair_kerning_shared(kern: &Kern<'_>, left: GlyphId, right: GlyphId) -> i32 {
    kern.subtables()
        .flatten()
        .filter(|st| st.is_horizontal() && !st.is_cross_stream() && !st.is_variable())
        .map(|st| match st.kind() {
            Ok(SubtableKind::Format0(t)) => t.kerning(left, right).unwrap_or(0),
            Ok(SubtableKind::Format2(t)) => t.kerning(left, right).unwrap_or(0),
            Ok(SubtableKind::Format3(t)) => t.kerning(left, right).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn collect_gpos_kern_lookups(gpos: &Gpos<'_>) -> Vec<u16> {
    let mut kern_lookups: Vec<u16> = Vec::new();
    if let Ok(feature_list) = gpos.feature_list() {
        let data = feature_list.offset_data();
        for rec in feature_list.feature_records() {
            if rec.feature_tag() == Tag::new(b"kern")
                && let Ok(feat) = rec.feature(data)
            {
                for idx in feat.lookup_list_indices() {
                    let i = idx.get();
                    if !kern_lookups.contains(&i) {
                        kern_lookups.push(i);
                    }
                }
            }
        }
    }
    kern_lookups.sort_unstable();
    if kern_lookups.is_empty()
        && let Ok(lookup_list) = gpos.lookup_list()
    {
        return (0..lookup_list.lookup_count()).collect();
    }
    kern_lookups
}

fn gpos_pair_kerning(gpos: &Gpos<'_>, lookup_indices: &[u16], left: GlyphId, right: GlyphId) -> i32 {
    let Ok(lookup_list) = gpos.lookup_list() else {
        return 0;
    };
    let Ok(right16) = GlyphId16::try_from(right) else {
        return 0;
    };
    let mut total = 0i32;
    let lookups = lookup_list.lookups();
    for &idx in lookup_indices {
        let Ok(lookup) = lookups.get(idx as usize) else {
            continue;
        };
        let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else {
            continue;
        };
        for sub in subtables.iter().flatten() {
            if let Some(adj) = pair_pos_kerning(&sub, left, right, right16) {
                total += adj;
                break;
            }
        }
    }
    total
}

fn pair_pos_kerning(
    sub: &PairPos<'_>,
    left: GlyphId,
    right: GlyphId,
    right16: GlyphId16,
) -> Option<i32> {
    match sub {
        PairPos::Format1(t) => {
            let cov_idx = t.coverage().ok()?.get(left)?;
            let pair_set = t.pair_sets().get(cov_idx as usize).ok()?;
            let records = pair_set.pair_value_records();
            let mut lo = 0usize;
            let mut hi = records.len();
            let mut found = None;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let rec = records.get(mid).ok()?;
                match rec.second_glyph().cmp(&right16) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => {
                        found = Some(rec);
                        break;
                    }
                }
            }
            let rec = found?;
            Some(rec.value_record1().x_advance().unwrap_or(0) as i32)
        }
        PairPos::Format2(t) => {
            t.coverage().ok()?.get(left)?;
            let c1 = t.class_def1().ok()?.get(left);
            let c2 = t.class_def2().ok()?.get(right);
            let rec1 = t.class1_records().get(c1 as usize).ok()?;
            let rec2 = rec1.class2_records().get(c2 as usize).ok()?;
            Some(rec2.value_record1().x_advance().unwrap_or(0) as i32)
        }
    }
}

/// Metrics provider for targets without direct filesystem access to `/System/Library/Fonts` (WASM),
/// backed by any fonts registered via [`register_memory_font`].
pub struct NoFontMetrics;

impl FontMetrics for NoFontMetrics {
    fn has_family(&self, family: &str) -> bool {
        let key = family.trim().to_ascii_lowercase();
        memory_registry()
            .lock()
            .ok()
            .is_some_and(|m| m.get(&key).is_some_and(|v| !v.is_empty()))
    }

    fn char_steps(&self, family: &str, bold: bool, italic: bool, size_pt: f32, text: &str) -> Option<Vec<f32>> {
        let (data, index) = memory_face_file(family, bold, italic)?;
        char_steps_from_bytes(&data, index, size_pt, text)
    }
}

/// Families shipped with every macOS install, used to recognise PDF fonts when no
/// [`FontMetrics`] can look at the installed fonts (WASM).
const MAC_SYSTEM_FAMILIES: &[&str] = &[
    "American Typewriter",
    "Andale Mono",
    "Arial",
    "Arial Black",
    "Arial Narrow",
    "Arial Rounded MT Bold",
    "Avenir",
    "Avenir Next",
    "Avenir Next Condensed",
    "Baskerville",
    "Big Caslon",
    "Bodoni 72",
    "Bradley Hand",
    "Brush Script MT",
    "Chalkboard",
    "Charter",
    "Cochin",
    "Comic Sans MS",
    "Copperplate",
    "Courier",
    "Courier New",
    "Didot",
    "DIN Alternate",
    "DIN Condensed",
    "Futura",
    "Geneva",
    "Georgia",
    "Gill Sans",
    "Helvetica",
    "Helvetica Neue",
    "Hoefler Text",
    "Impact",
    "Lucida Grande",
    "Marker Felt",
    "Menlo",
    "Microsoft Sans Serif",
    "Monaco",
    "Optima",
    "Palatino",
    "Papyrus",
    "PT Mono",
    "PT Sans",
    "PT Serif",
    "Rockwell",
    "Skia",
    "Symbol",
    "Tahoma",
    "Times",
    "Times New Roman",
    "Trebuchet MS",
    "Verdana",
    "Zapf Dingbats",
    "Zapfino",
];

/// Style words that PDF producers glue to the family name (`NunitoBold`, `Montserrat-LightItalic`).
const STYLE_WORDS: &[&str] = &[
    "GrasItalique",
    "BoldItalic",
    "BoldOblique",
    "SemiBold",
    "Semibold",
    "DemiBold",
    "ExtraBold",
    "ExtraLight",
    "UltraLight",
    "Italique",
    "Regular",
    "Normal",
    "Neutral",
    "Book",
    "Medium",
    "Light",
    "Slim",
    "Thin",
    "Black",
    "Heavy",
    "Bold",
    "Gras",
    "Italic",
    "Oblique",
];

/// Vendor suffixes on PostScript names (`ArialMT`, `TimesNewRomanPSMT`, `TradeGothicLTStd`).
const VENDOR_SUFFIXES: &[&str] = &["PSMT", "LTStd", "LTPro", "Std", "Pro", "MT", "PS", "LT"];

/// Mac family names that may reproduce `font`, most specific first:
/// `Arial-BoldMT` → `Arial`, `TimesNewRomanPSMT` → `Times New Roman`, `NunitoBold` → `Nunito`.
pub fn pdf_font_family_candidates(font: &FontIr) -> Vec<String> {
    let raw = if font.family_name.is_empty() {
        font.postscript_name.as_str()
    } else {
        font.family_name.as_str()
    };
    let raw = match raw.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 => rest,
        _ => raw,
    };
    let normalized = raw
        .replace("-Sans", "Sans")
        .replace("-Serif", "Serif")
        .replace("-Mono", "Mono");
    let mut base = normalized
        .split(['-', ',', '_'])
        .next()
        .unwrap_or(&normalized)
        .trim()
        .to_string();
    loop {
        let before = base.len();
        for suffix in VENDOR_SUFFIXES.iter().chain(STYLE_WORDS) {
            if base.len() > suffix.len() + 2 && base.ends_with(suffix) {
                base.truncate(base.len() - suffix.len());
                break;
            }
        }
        if base.len() == before {
            break;
        }
    }

    let spaced = split_camel_case(&base);
    let mut out = vec![spaced.clone()];
    if base != spaced {
        out.push(base.clone());
    }
    match spaced.as_str() {
        "Times" => out.push("Times New Roman".to_string()),
        "Liberation Sans" => out.push("Arial".to_string()),
        "Liberation Serif" => out.push("Times New Roman".to_string()),
        "Liberation Mono" => out.push("Courier New".to_string()),
        "Zapf Dingbats" | "ZapfDingbats" => out.push("Zapf Dingbats".to_string()),
        _ => {}
    }
    out
}

/// `TimesNewRoman` → `Times New Roman`, `ComicSansMS` → `Comic Sans MS`.
fn split_camel_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || (prev.is_uppercase() && next_lower) {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

/// Non-default weight subfamilies (`Montserrat-Light` → `Light`, `Inter-Medium` → `Medium`,
/// `NotoSans-Black` → `Black`, `OpenSans-SemiBold` → `SemiBold`) that OOXML cannot select with
/// the boolean `<w:b/>` flag alone and therefore need their own subfamily-qualified family name
/// (`Montserrat Light`, `Inter Medium`, `Noto Sans Black`, `Open Sans SemiBold`) when installed.
pub fn pdf_font_weight_subfamily(font: &FontIr) -> Option<&'static str> {
    let raw = if font.family_name.is_empty() {
        font.postscript_name.as_str()
    } else {
        font.family_name.as_str()
    };
    let tail = raw.split_once('+').map_or(raw, |(_, rest)| rest);
    for (needle, label) in [
        ("ExtraBold", "ExtraBold"),
        ("UltraBold", "ExtraBold"),
        ("SemiBold", "SemiBold"),
        ("Semibold", "SemiBold"),
        ("DemiBold", "SemiBold"),
        ("Black", "Black"),
        ("Heavy", "Black"),
    ] {
        if tail.contains(needle) {
            return Some(label);
        }
    }
    if font.is_bold {
        return None;
    }
    for (needle, label) in [
        ("ExtraLight", "ExtraLight"),
        ("UltraLight", "ExtraLight"),
        ("Light", "Light"),
        ("Thin", "Thin"),
        ("Medium", "Medium"),
    ] {
        if tail.contains(needle) {
            return Some(label);
        }
    }
    None
}

/// The installed family that reproduces `font`, and whether it is the PDF's own family
/// (metrics identical) rather than a look-alike substitute.
pub fn resolve_exact_font(font: &FontIr, metrics: &dyn FontMetrics, profile: Profile) -> (String, bool) {
    let candidates = pdf_font_family_candidates(font);
    if let Some(sub) = pdf_font_weight_subfamily(font) {
        for cand in &candidates {
            let sub_family = format!("{cand} {sub}");
            if metrics.has_family(&sub_family) {
                return (sub_family, true);
            }
        }
    }
    for cand in candidates {
        let installed = metrics.has_family(&cand) || MAC_SYSTEM_FAMILIES.iter().any(|f| f.eq_ignore_ascii_case(&cand));
        if installed {
            return (cand, true);
        }
    }
    let family = metrics
        .closest_family(font)
        .unwrap_or_else(|| map_pdf_font_to_mac(font, profile).0);
    (family, false)
}

#[cfg(not(target_arch = "wasm32"))]
pub use system::SystemFontMetrics;

#[cfg(not(target_arch = "wasm32"))]
mod system {
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Mutex, OnceLock};

    use skrifa::instance::{LocationRef, Size};
    use skrifa::raw::FileRef;
    use skrifa::string::StringId;
    use skrifa::{FontRef, MetadataProvider};

    use p2d_ir::{FontIr, PathOp};
    use skrifa::outline::{DrawSettings, OutlinePen};

    use super::{FontMetrics, MAC_SYSTEM_FAMILIES};
    use crate::lookalike;

    /// Collects an outline as [`PathOp`]s.
    struct OpsPen(Vec<PathOp>);

    impl OutlinePen for OpsPen {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push(PathOp::MoveTo([x, y]));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push(PathOp::LineTo([x, y]));
        }
        fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
            self.0.push(PathOp::QuadTo([cx, cy, x, y]));
        }
        fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
            self.0.push(PathOp::CurveTo([ax, ay, bx, by, x, y]));
        }
        fn close(&mut self) {
            self.0.push(PathOp::Close);
        }
    }

    const FONT_DIRS: &[&str] = &[
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
    ];

    #[derive(Clone)]
    struct Face {
        data: Arc<Vec<u8>>,
        index: u32,
        bold: bool,
        italic: bool,
        weight: f32,
    }

    /// [`FontMetrics`] backed by the font files installed on this Mac, automatically downloading
    /// and installing missing open-source families from Google Fonts into `~/Library/Fonts`.
    pub struct SystemFontMetrics {
        files: Mutex<Vec<PathBuf>>,
        file_bytes: Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>,
        families: Mutex<HashMap<String, Vec<Face>>>,
        /// Look-alike found for each missing font (by PostScript name and style).
        lookalikes: Mutex<HashMap<String, Option<String>>>,
        cand_glyphs: Mutex<HashMap<(String, bool, bool, char), Option<(Vec<PathOp>, f32)>>>,
    }

    impl Default for SystemFontMetrics {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SystemFontMetrics {
        /// Shared process-wide metrics instance so loaded font files and look-alike scores are
        /// reused across conversions and edit sessions.
        pub fn shared() -> &'static Self {
            static SHARED: OnceLock<SystemFontMetrics> = OnceLock::new();
            SHARED.get_or_init(Self::new)
        }

        pub fn new() -> Self {
            let mut dirs: Vec<PathBuf> = FONT_DIRS.iter().map(PathBuf::from).collect();
            if let Ok(home) = std::env::var("HOME") {
                dirs.push(Path::new(&home).join("Library/Fonts"));
            }
            let mut files = Vec::new();
            for dir in dirs {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                        files.push(path);
                    }
                }
            }
            Self {
                files: Mutex::new(files),
                file_bytes: Mutex::new(HashMap::new()),
                families: Mutex::new(HashMap::new()),
                lookalikes: Mutex::new(HashMap::new()),
                cand_glyphs: Mutex::new(HashMap::new()),
            }
        }

        fn scan_files(&self, family: &str) -> Vec<Face> {
            let key = family.to_ascii_lowercase();
            let squashed: String = key.chars().filter(|c| c.is_alphanumeric()).collect();
            let first_word: String = key
                .split(' ')
                .next()
                .unwrap_or("")
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            let paths = self.files.lock().map(|f| f.clone()).unwrap_or_default();
            let mut faces = Vec::new();
            for path in &paths {
                let file_key: String = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect();
                if matches!(
                    file_key.as_str(),
                    "seravek" | "iowanoldstyle" | "athelas" | "marion" | "superclarendon"
                ) {
                    continue;
                }
                if file_key == "arialunicode" && !squashed.starts_with("arialunicode") {
                    continue;
                }
                if !file_key.contains(&squashed) && !(first_word.len() >= 4 && file_key.starts_with(&first_word)) {
                    continue;
                }
                let data = if let Some(cached) = self.file_bytes.lock().ok().and_then(|m| m.get(path).cloned()) {
                    cached
                } else {
                    let Ok(bytes) = std::fs::read(path) else {
                        continue;
                    };
                    let arc = Arc::new(bytes);
                    if let Ok(mut m) = self.file_bytes.lock() {
                        m.insert(path.clone(), arc.clone());
                    }
                    arc
                };
                let Ok(file) = FileRef::new(&data) else {
                    continue;
                };
                for (index, font) in file.fonts().enumerate() {
                    let Ok(font) = font else {
                        continue;
                    };
                    let names = [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME];
                    let matches = names.iter().any(|&id| {
                        font.localized_strings(id)
                            .english_or_first()
                            .is_some_and(|s| s.to_string().eq_ignore_ascii_case(family))
                    });
                    if !matches {
                        continue;
                    }
                    let attrs = font.attributes();
                    let weight = attrs.weight.value();
                    faces.push(Face {
                        data: data.clone(),
                        index: index as u32,
                        bold: weight >= 600.0,
                        italic: attrs.style != skrifa::attribute::Style::Normal,
                        weight,
                    });
                }
            }
            faces
        }

        fn faces(&self, family: &str) -> Vec<Face> {
            let key = family.to_ascii_lowercase();
            if let Some(found) = self.families.lock().ok().and_then(|m| m.get(&key).cloned()) {
                return found;
            }
            let mut faces = self.scan_files(family);
            if faces.is_empty() && should_try_google_fonts(family) {
                let installed = install_google_font_family(family);
                if !installed.is_empty() {
                    if let Ok(mut list) = self.files.lock() {
                        for p in installed {
                            if !list.contains(&p) {
                                list.push(p);
                            }
                        }
                    }
                    faces = self.scan_files(family);
                }
            }
            if let Ok(mut m) = self.families.lock() {
                m.insert(key, faces.clone());
            }
            faces
        }

        /// The installed file holding the face of `family` closest to `bold` / `italic`, and the
        /// index of that face in it (font collections hold several).
        pub fn face_file(&self, family: &str, bold: bool, italic: bool) -> Option<(Arc<Vec<u8>>, u32)> {
            self.pick(family, bold, italic).map(|f| (f.data, f.index))
        }

        fn pick(&self, family: &str, bold: bool, italic: bool) -> Option<Face> {
            let faces = self.faces(family);
            let target = if bold { 700.0 } else { 400.0 };
            faces.into_iter().min_by(|a, b| {
                let score = |f: &Face| {
                    let mut s = (f.weight - target).abs();
                    if f.bold != bold {
                        s += 1000.0;
                    }
                    if f.italic != italic {
                        s += 2000.0;
                    }
                    s
                };
                score(a).total_cmp(&score(b))
            })
        }
    }

    impl FontMetrics for SystemFontMetrics {
        fn has_family(&self, family: &str) -> bool {
            !self.faces(family).is_empty()
        }

        fn closest_family(&self, font: &FontIr) -> Option<String> {
            let key = format!("{}|{}|{}", font.postscript_name, font.is_bold, font.is_italic);
            if let Some(found) = self.lookalikes.lock().ok().and_then(|m| m.get(&key).cloned()) {
                return found;
            }
            let samples = lookalike::samples(font);
            let best = if samples.len() < 3 {
                None
            } else {
                lookalike::CANDIDATES
                    .iter()
                    .filter_map(|family| {
                        // Only a family with a real face in this style: Pages would fake the others.
                        let face = self
                            .pick(family, font.is_bold, font.is_italic)
                            .filter(|f| f.bold == font.is_bold && f.italic == font.is_italic)?;
                        let f = FontRef::from_index(&face.data, face.index).ok()?;
                        let charmap = f.charmap();
                        let outlines = f.outline_glyphs();
                        let size = Size::new(1000.0);
                        let metrics = f.glyph_metrics(size, LocationRef::default());
                        let other_class = lookalike::SERIF.contains(family) != font.is_serif;
                        let penalty = if other_class { lookalike::CLASS_PENALTY } else { 0.0 };
                        let score = -penalty + lookalike::score(&samples, |c| {
                            let cache_key = ((*family).to_string(), font.is_bold, font.is_italic, c);
                            if let Some(cached) = self.cand_glyphs.lock().ok().and_then(|m| m.get(&cache_key).cloned()) {
                                return cached;
                            }
                            let computed = (|| {
                                let gid = charmap.map(c)?;
                                let mut pen = OpsPen(Vec::new());
                                outlines
                                    .get(gid)?
                                    .draw(DrawSettings::unhinted(size, LocationRef::default()), &mut pen)
                                    .ok()?;
                                Some((pen.0, metrics.advance_width(gid)?))
                            })();
                            if let Ok(mut m) = self.cand_glyphs.lock() {
                                m.insert(cache_key, computed.clone());
                            }
                            computed
                        });
                        Some((family.to_string(), score))
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .filter(|(_, score)| *score > 0.3)
                    .map(|(family, _)| family)
            };
            if let Ok(mut m) = self.lookalikes.lock() {
                m.insert(key, best.clone());
            }
            best
        }

        fn char_steps(&self, family: &str, bold: bool, italic: bool, size_pt: f32, text: &str) -> Option<Vec<f32>> {
            let face = self.pick(family, bold, italic)?;
            super::char_steps_from_bytes(&face.data, face.index, size_pt, text)
        }
    }

    fn google_base_family(family: &str) -> &str {
        for suffix in [
            " ExtraLight",
            " Light",
            " Thin",
            " Medium",
            " SemiBold",
            " ExtraBold",
            " Black",
        ] {
            if let Some(base) = family.strip_suffix(suffix) {
                return base.trim();
            }
        }
        family.trim()
    }

    fn should_try_google_fonts(family: &str) -> bool {
        #[cfg(test)]
        if std::env::var("P2D_ALLOW_GOOGLE_FONTS_IN_TESTS").is_err() {
            return false;
        }
        let base = google_base_family(family);
        let lower = base.to_ascii_lowercase();
        base.len() >= 3
            && base.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 3
            && !matches!(
                lower.as_str(),
                "font" | "default" | "unknown" | "sans" | "serif" | "monospace" | "none"
            )
            && !MAC_SYSTEM_FAMILIES.iter().any(|f| f.eq_ignore_ascii_case(base))
            && !base.starts_with("CIDFont")
            && !base.starts_with("p2d-")
    }

    /// Downloads a missing family from Google Fonts (`fonts.googleapis.com/css`), normalizes the
    /// `name`/`OS/2`/`head` tables of each weight/style `.ttf` so macOS CoreText, Apple Pages, and
    /// Word register it under the exact expected family name (`Montserrat`, `Montserrat Light`,
    /// `Nunito`, `Oswald`, …), and installs the `.ttf` files into `~/Library/Fonts`.
    fn install_google_font_family(family: &str) -> Vec<PathBuf> {
        static ATTEMPTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
        let base = google_base_family(family);
        let base_key = base.to_ascii_lowercase();
        let key = family.trim().to_ascii_lowercase();
        let Ok(home) = std::env::var("HOME") else {
            return Vec::new();
        };
        let fonts_dir = Path::new(&home).join("Library/Fonts");
        let _ = std::fs::create_dir_all(&fonts_dir);
        let cache_file = fonts_dir.join(".p2d-attempted-families");
        let set = ATTEMPTED.get_or_init(|| {
            let mut initial = HashSet::new();
            if let Ok(contents) = std::fs::read_to_string(&cache_file) {
                for line in contents.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        initial.insert(trimmed.to_string());
                    }
                }
            }
            Mutex::new(initial)
        });
        if let Ok(mut guard) = set.lock() {
            if !guard.insert(key.clone()) {
                return Vec::new();
            }
            let persisted: String = guard.iter().map(|s| format!("{s}\n")).collect();
            let _ = std::fs::write(&cache_file, persisted);
        }

        let fetch_family = match base_key.as_str() {
            "calibri" => "Carlito",
            "cambria" => "Caladea",
            "consolas" => "Inconsolata",
            _ => base,
        };
        p2d_ir::progress::report(p2d_ir::progress::Step::Font { family: fetch_family.to_string() });
        let url = format!(
            "https://fonts.googleapis.com/css?family={}:300,400,500,600,700,800,900,300italic,400italic,500italic,600italic,700italic,800italic,900italic&subset=latin,latin-ext",
            fetch_family.replace(' ', "+")
        );
        let Ok(out) = Command::new("/usr/bin/curl")
            .args(["-fsSL", "--max-time", "5", &url])
            .output()
        else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        let css = String::from_utf8_lossy(&out.stdout);
        let slug: String = base.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let mut installed = Vec::new();

        for block in css.split("@font-face") {
            let italic = block.contains("font-style: italic");
            let Some(weight) = extract_after(block, "font-weight:")
                .and_then(|s| s.split(';').next())
                .and_then(|s| s.trim().parse::<u16>().ok())
            else {
                continue;
            };
            let Some(ttf_url) = extract_after(block, "url(")
                .and_then(|s| s.split(')').next())
                .map(|s| s.trim().trim_matches(['\'', '"']))
                .filter(|u| u.starts_with("https://") && u.ends_with(".ttf"))
            else {
                continue;
            };
            let (target_family, target_bold, weight_tag) = match weight {
                300 => (format!("{base} Light"), false, "Light"),
                400 => (base.to_string(), false, "Regular"),
                500 => (format!("{base} Medium"), false, "Medium"),
                600 => (format!("{base} SemiBold"), false, "SemiBold"),
                700 => (base.to_string(), true, "Bold"),
                800 => (format!("{base} ExtraBold"), false, "ExtraBold"),
                900 => (format!("{base} Black"), false, "Black"),
                _ => continue,
            };
            let style_suffix = match (weight_tag, italic) {
                ("Regular", true) => "Italic".to_string(),
                (tag, true) => format!("{tag}Italic"),
                (tag, false) => tag.to_string(),
            };
            let dest = fonts_dir.join(format!("{slug}-{style_suffix}.ttf"));
            if dest.exists() {
                register_macos_font(&dest);
                installed.push(dest);
                continue;
            }
            let Ok(ttf_out) = Command::new("/usr/bin/curl")
                .args(["-fsSL", "--max-time", "6", ttf_url])
                .output()
            else {
                continue;
            };
            if !ttf_out.status.success() || ttf_out.stdout.len() < 256 {
                continue;
            }
            let bytes = relabel_ttf(&ttf_out.stdout, &target_family, target_bold, italic)
                .unwrap_or(ttf_out.stdout);
            if std::fs::write(&dest, &bytes).is_ok() {
                register_macos_font(&dest);
                installed.push(dest);
            }
        }
        installed
    }

    #[cfg(target_os = "macos")]
    fn register_macos_font(path: &Path) {
        use std::os::unix::ffi::OsStrExt;
        #[link(name = "CoreText", kind = "framework")]
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFURLCreateFromFileSystemRepresentation(
                allocator: *const std::ffi::c_void,
                buffer: *const u8,
                buf_len: isize,
                is_directory: u8,
            ) -> *const std::ffi::c_void;
            fn CFRelease(cf: *const std::ffi::c_void);
            fn CTFontManagerRegisterFontsForURL(
                font_url: *const std::ffi::c_void,
                scope: u32,
                error: *mut *const std::ffi::c_void,
            ) -> bool;
        }
        let bytes = path.as_os_str().as_bytes();
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                0,
            );
            if !url.is_null() {
                // Scope 2 = User/Persistent (notifies running apps via fontd); fall back to Scope 1 = Process.
                if !CTFontManagerRegisterFontsForURL(url, 2, std::ptr::null_mut()) {
                    let _ = CTFontManagerRegisterFontsForURL(url, 1, std::ptr::null_mut());
                }
                CFRelease(url);
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn register_macos_font(_path: &Path) {}

    fn extract_after<'a>(haystack: &'a str, needle: &str) -> Option<&'a str> {
        let pos = haystack.find(needle)?;
        Some(&haystack[pos + needle.len()..])
    }

    /// Rebuilds the `name` table, strips `STAT`/`fvar` variable-origin overrides, and updates
    /// `OS/2` + `head` style bits so macOS CoreText registers the `.ttf` under `family`.
    pub(super) fn relabel_ttf(raw: &[u8], family: &str, bold: bool, italic: bool) -> Option<Vec<u8>> {
        if raw.len() < 12 {
            return None;
        }
        let version = u32::from_be_bytes(raw[0..4].try_into().ok()?);
        let num_tables = u16::from_be_bytes(raw[4..6].try_into().ok()?) as usize;
        if raw.len() < 12 + 16 * num_tables {
            return None;
        }
        let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::with_capacity(num_tables);
        for i in 0..num_tables {
            let rec = &raw[12 + 16 * i..12 + 16 * (i + 1)];
            let tag: [u8; 4] = rec[0..4].try_into().ok()?;
            if matches!(&tag, b"name" | b"STAT" | b"fvar") {
                continue;
            }
            let offset = u32::from_be_bytes(rec[8..12].try_into().ok()?) as usize;
            let len = u32::from_be_bytes(rec[12..16].try_into().ok()?) as usize;
            let mut data = raw.get(offset..offset.checked_add(len)?)?.to_vec();
            if &tag == b"OS/2" && data.len() >= 64 {
                let w: u16 = if bold { 700 } else { 400 };
                data[4..6].copy_from_slice(&w.to_be_bytes());
                let mut sel = u16::from_be_bytes([data[62], data[63]]) & !(0x0001 | 0x0020 | 0x0040 | 0x0100);
                if italic {
                    sel |= 0x0001;
                }
                if bold {
                    sel |= 0x0020;
                }
                if !bold && !italic {
                    sel |= 0x0040;
                }
                data[62..64].copy_from_slice(&sel.to_be_bytes());
            } else if &tag == b"head" && data.len() >= 46 {
                let mac_style: u16 = (if bold { 1 } else { 0 }) | (if italic { 2 } else { 0 });
                data[44..46].copy_from_slice(&mac_style.to_be_bytes());
            }
            tables.push((tag, data));
        }
        let subfamily = match (bold, italic) {
            (false, false) => "Regular",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (true, true) => "Bold Italic",
        };
        let ps_family: String = family.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let ps_sub: String = subfamily.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let ps_name = format!("{ps_family}-{ps_sub}");
        let full = if subfamily == "Regular" {
            family.to_string()
        } else {
            format!("{family} {subfamily}")
        };
        let records = [
            (1u16, family),
            (2, subfamily),
            (3, ps_name.as_str()),
            (4, full.as_str()),
            (5, "Version 1.0"),
            (6, ps_name.as_str()),
        ];
        let mut strings = Vec::new();
        let mut name_buf = Vec::new();
        for v in [0u16, records.len() as u16, 6 + 12 * records.len() as u16] {
            name_buf.extend_from_slice(&v.to_be_bytes());
        }
        for (id, text) in records {
            let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
            for v in [3u16, 1, 0x0409, id, utf16.len() as u16, strings.len() as u16] {
                name_buf.extend_from_slice(&v.to_be_bytes());
            }
            strings.extend_from_slice(&utf16);
        }
        name_buf.extend_from_slice(&strings);
        tables.push((*b"name", name_buf));

        tables.sort_by_key(|(tag, _)| *tag);
        for (tag, data) in &mut tables {
            if tag == b"head" && data.len() >= 12 {
                data[8..12].copy_from_slice(&[0; 4]);
            }
        }
        let n = tables.len() as u16;
        let pow = 1u16 << (15 - n.max(1).leading_zeros() as u16);
        let mut out = Vec::new();
        out.extend_from_slice(&version.to_be_bytes());
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(&(pow * 16).to_be_bytes());
        out.extend_from_slice(&(pow.trailing_zeros() as u16).to_be_bytes());
        out.extend_from_slice(&(n * 16 - pow * 16).to_be_bytes());
        let mut offset = 12 + 16 * tables.len();
        let mut head_at = None;
        for (tag, data) in &tables {
            if tag == b"head" {
                head_at = Some(offset);
            }
            out.extend_from_slice(tag);
            out.extend_from_slice(&sfnt_checksum(data).to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            offset += data.len().div_ceil(4) * 4;
        }
        for (_, data) in &tables {
            out.extend_from_slice(data);
            out.resize(out.len().div_ceil(4) * 4, 0);
        }
        if let Some(at) = head_at {
            let adjust = 0xB1B0_AFBAu32.wrapping_sub(sfnt_checksum(&out));
            out[at + 8..at + 12].copy_from_slice(&adjust.to_be_bytes());
        }
        Some(out)
    }

    fn sfnt_checksum(data: &[u8]) -> u32 {
        data.chunks(4).fold(0u32, |acc, c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            acc.wrapping_add(u32::from_be_bytes(w))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(name: &str) -> FontIr {
        FontIr {
            postscript_name: name.to_string(),
            family_name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_pdf_font_family_candidates() {
        assert_eq!(pdf_font_family_candidates(&font("Arial-BoldMT"))[0], "Arial");
        assert_eq!(
            pdf_font_family_candidates(&font("TimesNewRomanPSMT"))[0],
            "Times New Roman"
        );
        assert_eq!(
            pdf_font_family_candidates(&font("TimesNewRomanPS-BoldMT"))[0],
            "Times New Roman"
        );
        assert_eq!(pdf_font_family_candidates(&font("Helvetica-Oblique"))[0], "Helvetica");
        assert!(pdf_font_family_candidates(&font("Times-Roman")).contains(&"Times".to_string()));
        assert_eq!(pdf_font_family_candidates(&font("NunitoBold"))[0], "Nunito");
        assert_eq!(
            pdf_font_family_candidates(&font("TradeGothicLTStd-Bold"))[0],
            "Trade Gothic"
        );
        assert_eq!(
            pdf_font_family_candidates(&font("ABCDEF+Montserrat-Light"))[0],
            "Montserrat"
        );
        assert_eq!(
            pdf_font_family_candidates(&font("HelveticaNeue-Bold"))[0],
            "Helvetica Neue"
        );
        assert_eq!(pdf_font_family_candidates(&font("ComicSansMS"))[0], "Comic Sans MS");
    }

    #[test]
    fn test_resolve_exact_font_without_system_metrics() {
        let (fam, same) = resolve_exact_font(&font("Helvetica-Bold"), &NoFontMetrics, Profile::Pages);
        assert_eq!(fam, "Helvetica");
        assert!(same);
        let (_, same) = resolve_exact_font(&font("Montserrat-Light"), &NoFontMetrics, Profile::Pages);
        assert!(!same);
    }
}
