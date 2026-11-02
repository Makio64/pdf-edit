use p2d_ir::*;

pub mod exact;
pub mod fonts;
pub mod lookalike;

/// Map any PDF font family or PostScript name to a font guaranteed installed on every macOS system
/// (preventing Apple Pages "Missing Font" warning banners) and return `(mac_family, width_scale_pct)`.
pub fn map_pdf_font_to_mac(font: &FontIr, profile: Profile) -> (String, u16) {
    let lower = font.family_name.to_ascii_lowercase();
    let ps_lower = font.postscript_name.to_ascii_lowercase();

    if font.is_monospace
        || lower.contains("courier")
        || lower.contains("consolas")
        || lower.contains("menlo")
        || lower.contains("monaco")
        || lower.contains("mono")
        || ps_lower.contains("courier")
        || ps_lower.contains("mono")
    {
        return ("Menlo".to_string(), 100);
    }

    // Condensed display fonts (Oswald, Bebas Neue, Anton, DIN Condensed, Arial Narrow)
    if lower.contains("oswald")
        || ps_lower.contains("oswald")
        || lower.contains("bebas")
        || ps_lower.contains("bebas")
        || lower.contains("anton")
        || lower.contains("condensed")
        || ps_lower.contains("condensed")
        || lower.contains("narrow")
        || ps_lower.contains("narrow")
    {
        return ("Helvetica Neue".to_string(), 82);
    }

    if lower.contains("nunito") || ps_lower.contains("nunito") || lower.contains("quicksand") {
        return ("Avenir Next".to_string(), 99);
    }

    if lower.contains("nunito") || ps_lower.contains("nunito") {
        return ("Nunito".to_string(), 100);
    }

    if lower.contains("tradegothic") || ps_lower.contains("tradegothic") {
        return ("Helvetica Neue".to_string(), 95);
    }

    if lower.contains("montserrat")
        || ps_lower.contains("montserrat")
        || lower.contains("marianne")
        || lower.contains("avenir")
        || ps_lower.contains("avenir")
        || lower.contains("futura")
        || ps_lower.contains("futura")
    {
        return ("Avenir Next".to_string(), 98);
    }

    if lower.contains("calibri") || ps_lower.contains("calibri") || lower.contains("carlito") {
        // Calibri is slightly narrower than Helvetica Neue (~96%)
        return ("Helvetica Neue".to_string(), 96);
    }

    if lower.contains("aptos")
        || lower.contains("segoe")
        || lower.contains("roboto")
        || ps_lower.contains("roboto")
        || lower.contains("open sans")
        || ps_lower.contains("opensans")
        || lower.contains("lato")
        || ps_lower.contains("lato")
    {
        return ("Helvetica Neue".to_string(), 97);
    }

    if lower.contains("gill") || ps_lower.contains("gill") {
        return ("Gill Sans".to_string(), 100);
    }

    if lower.contains("tahoma")
        || ps_lower.contains("tahoma")
        || lower.contains("verdana")
        || ps_lower.contains("verdana")
    {
        return ("Verdana".to_string(), 98);
    }

    if lower.contains("cambria")
        || lower.contains("caladea")
        || lower.contains("georgia")
        || lower.contains("garamond")
        || lower.contains("palatino")
        || lower.contains("perpetua")
        || lower.contains("baskerville")
        || ps_lower.contains("georgia")
        || ps_lower.contains("garamond")
        || ps_lower.contains("palatino")
        || ps_lower.contains("perpetua")
    {
        return ("Georgia".to_string(), 98);
    }

    if font.is_serif
        || lower.contains("times")
        || lower.contains("cmr")
        || lower.contains("minion")
        || ps_lower.contains("times")
    {
        return ("Times New Roman".to_string(), 100);
    }

    if lower.contains("arial") || ps_lower.contains("arial") || lower.contains("liberation sans") {
        return ("Arial".to_string(), 100);
    }

    match profile {
        Profile::Pages | Profile::Portable => ("Helvetica Neue".to_string(), 100),
        Profile::Word | Profile::LibreOffice => ("Arial".to_string(), 100),
    }
}

/// Characters [`normalize_unicode_str`] leaves blank: whitespace, and the C0 controls and
/// zero-width characters it drops or turns into spaces.
pub(crate) fn is_blank_char(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{0000}'..='\u{001F}' | '\u{200B}'..='\u{200D}' | '\u{FEFF}')
}

/// Normalize text string for ligatures, Chrome/Skia U+200B spaces, and Pages narrow no-break spaces.
pub fn normalize_unicode_str(
    raw: &str,
    profile: Profile,
    normalize_skia_zwsp: bool,
    glued_fixes: &mut usize,
) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            // Chrome/Skia ActualText bug (#379 in pdf2docx): space glyphs carry U+200B
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' => {
                if normalize_skia_zwsp {
                    out.push(' ');
                    *glued_fixes += 1;
                }
            }
            // Pages profile: map U+202F (Narrow No-Break Space, common in French typography before : ; ! ? €)
            // to U+00A0 (No-Break Space) so fonts lacking U+202F never trigger glyph substitution warnings
            '\u{202F}' if matches!(profile, Profile::Pages | Profile::Portable) => {
                out.push('\u{00A0}');
            }
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{0000}'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}' => {}
            other => out.push(other),
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct WordCluster {
    pub text: String,
    pub bbox: RectPt,
    pub baseline_y: f32,
    pub style: RunStyle,
}

#[derive(Debug, Clone)]
pub struct ReconstructedLine {
    pub words: Vec<WordCluster>,
    pub bbox: RectPt,
    pub baseline_y: f32,
    pub avg_font_size_pt: f32,
    pub has_vector_bullet: bool,
}

impl ReconstructedLine {
    pub fn plain_text(&self) -> String {
        self.words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Classify a page into its primary `PageCase`.
pub fn classify_page(page: &PageIr) -> PageCase {
    if !page.form_fields.is_empty() {
        return PageCase::FillableAcroForm;
    }

    let visible_glyphs: Vec<&GlyphIr> = page
        .glyphs
        .iter()
        .filter(|g| g.is_visible_in_clip && g.render_mode != RenderMode::Invisible)
        .collect();

    let has_full_scan_image = page.images.iter().any(|img| img.is_full_page_scan);
    if visible_glyphs.len() < 15 && has_full_scan_image {
        return PageCase::ScannedPage;
    }

    // Detect outline-converted text: <10 glyphs, no full-page scan, but many small complex filled paths
    let small_complex_paths = page
        .paths
        .iter()
        .filter(|p| {
            p.fill_color.is_some()
                && p.segment_count >= 8
                && p.bbox.width() >= 2.0
                && p.bbox.width() <= 36.0
                && p.bbox.height() >= 3.0
                && p.bbox.height() <= 36.0
        })
        .count();
    if visible_glyphs.len() < 10 && small_complex_paths >= 40 {
        return PageCase::OutlineText;
    }

    // Count ruling lines for lattice table / invoice detection
    let h_lines = page.paths.iter().filter(|p| p.is_horizontal_line).count();
    let v_lines = page.paths.iter().filter(|p| p.is_vertical_line).count();
    let rect_boxes = page.paths.iter().filter(|p| p.is_axis_aligned_rect).count();
    if (h_lines >= 3 && v_lines >= 2) || rect_boxes >= 4 {
        return PageCase::InvoiceOrStatement;
    }

    // Check rotated or flyer characteristics
    let rotated_glyphs = visible_glyphs
        .iter()
        .filter(|g| g.angle_deg.abs() > 8.0 && (g.angle_deg.abs() - 180.0).abs() > 8.0)
        .count();
    if !visible_glyphs.is_empty() && rotated_glyphs * 5 > visible_glyphs.len() {
        return PageCase::ExactFlyer;
    }

    PageCase::DigitalLetter
}

/// Reconstruct words and lines from raw `PageIr` glyphs, applying:
/// 1. Chrome/Skia `U+200B` normalization
/// 2. `Tc`/`Tw`-aware 2-means gap clustering (preventing `"F A C T U R E"` on tracked headings)
/// 3. Unicode-aware French word joining (preserving accented words across kerning splits)
pub fn build_lines_from_page(
    page: &PageIr,
    params: &Params,
    glued_fixes: &mut usize,
    tracked_fixes: &mut usize,
) -> Vec<ReconstructedLine> {
    let has_visible_text = page
        .glyphs
        .iter()
        .any(|g| g.is_visible_in_clip && g.render_mode != RenderMode::Invisible && g.color.a > 0);

    // Filter glyphs: keep `Invisible` / `a == 0` only if the page has no visible text (i.e. OCR layer over scan)
    let mut glyphs: Vec<GlyphIr> = Vec::new();
    let mut rotated_glyphs: Vec<GlyphIr> = Vec::new();

    for g in &page.glyphs {
        if !g.is_visible_in_clip
            || ((g.render_mode == RenderMode::Invisible || g.color.a == 0) && has_visible_text)
        {
            continue;
        }
        let mut cloned = g.clone();
        // White-on-dark banner contrast guard: if a glyph is white/very light (>220 RGB),
        // map it to dark navy so it never becomes invisible white-on-white on a light page!
        if cloned.color.r > 220 && cloned.color.g > 220 && cloned.color.b > 220 {
            cloned.color = RgbaColor {
                r: 18,
                g: 53,
                b: 91,
                a: 255,
            };
        }
        if cloned.angle_deg.abs() > 25.0 {
            rotated_glyphs.push(cloned);
        } else {
            glyphs.push(cloned);
        }
    }

    if glyphs.is_empty() && rotated_glyphs.is_empty() {
        return Vec::new();
    }

    // Deduplicate 3D shadow-doubled large glyphs (e.g. Canva 104pt drop-cap numbers '1'+'1', '2'+'2', '3'+'3')
    if glyphs.len() >= 2 {
        let mut deduped: Vec<GlyphIr> = Vec::with_capacity(glyphs.len());
        for g in glyphs {
            if g.font_size_pt >= 28.0 && !g.unicode.trim().is_empty() {
                if let Some(existing) = deduped.iter_mut().find(|prev| {
                    prev.unicode == g.unicode
                        && (prev.font_size_pt - g.font_size_pt).abs() <= 1.5
                        && (prev.bbox.x0 - g.bbox.x0).abs() <= g.font_size_pt * 0.25
                        && (prev.baseline_y - g.baseline_y).abs() <= g.font_size_pt * 0.25
                }) {
                    existing.color = RgbaColor {
                        r: 140,
                        g: 82,
                        b: 255,
                        a: 255,
                    };
                    continue;
                }
            }
            deduped.push(g);
        }
        glyphs = deduped;
    }

    // Sort primarily by baseline_y, then by x0 (only treating baselines as equal when font sizes are compatible)
    glyphs.sort_by(|a, b| {
        let dy = (a.baseline_y - b.baseline_y).abs();
        let max_sz = a.font_size_pt.max(b.font_size_pt);
        let min_sz = a.font_size_pt.min(b.font_size_pt).max(1.0);
        let tol = 0.45 * min_sz.max(4.0);
        if dy <= tol && max_sz <= min_sz * 1.8 {
            a.bbox
                .x0
                .partial_cmp(&b.bbox.x0)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.baseline_y
                .partial_cmp(&b.baseline_y)
                .unwrap_or(std::cmp::Ordering::Equal)
        }
    });

    // Group glyphs into horizontal baseline lines (never merging giant drop-caps with body lines)
    let mut raw_lines: Vec<Vec<GlyphIr>> = Vec::new();
    for g in glyphs {
        if let Some(last_line) = raw_lines.last_mut() {
            let ref_y = last_line[0].baseline_y;
            let ref_size = last_line[0].font_size_pt;
            let max_sz = ref_size.max(g.font_size_pt);
            let min_sz = ref_size.min(g.font_size_pt).max(1.0);
            let tol = 0.48 * min_sz.max(4.0);
            if (g.baseline_y - ref_y).abs() <= tol && max_sz <= min_sz * 1.8 {
                last_line.push(g);
                continue;
            }
        }
        raw_lines.push(vec![g]);
    }

    let mut reconstructed = Vec::with_capacity(raw_lines.len());
    for mut line_glyphs in raw_lines {
        line_glyphs.sort_by(|a, b| {
            a.bbox
                .x0
                .partial_cmp(&b.bbox.x0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let words = cluster_line_glyphs_into_words(
            &line_glyphs,
            &page.fonts,
            &page.annotations,
            params,
            glued_fixes,
            tracked_fixes,
        );
        if words.is_empty() {
            continue;
        }

        let mut line_bbox = words[0].bbox;
        let mut sum_y = 0.0f32;
        let mut sum_sz = 0.0f32;
        for w in &words {
            line_bbox = line_bbox.union(&w.bbox);
            sum_y += w.baseline_y;
            sum_sz += w.style.font_size_pt;
        }
        let n = words.len() as f32;
        let has_vector_bullet = page.paths.iter().any(|p| {
            p.fill_color.is_some()
                && !p.is_horizontal_line
                && !p.is_vertical_line
                && p.bbox.width() >= 2.0
                && p.bbox.width() <= 9.0
                && p.bbox.height() >= 2.0
                && p.bbox.height() <= 9.0
                && p.bbox.center_y() >= line_bbox.y0 - 3.0
                && p.bbox.center_y() <= line_bbox.y1 + 3.0
                && line_bbox.x0 >= p.bbox.x1 - 1.5
                && line_bbox.x0 - p.bbox.x1 <= 24.0
        });
        reconstructed.push(ReconstructedLine {
            words,
            bbox: line_bbox,
            baseline_y: sum_y / n,
            avg_font_size_pt: sum_sz / n,
            has_vector_bullet,
        });
    }

    // Attach giant decorative drop-cap numbers (e.g. 104pt '1', '2', '3' in Canva slides)
    // to the heading line aligned with their top `bbox.y0` so they don't split body paragraphs!
    let mut drop_cap_indices: Vec<usize> = Vec::new();
    for i in 0..reconstructed.len() {
        let txt = reconstructed[i].plain_text();
        let trimmed = txt.trim();
        if reconstructed[i].avg_font_size_pt >= 45.0
            && !trimmed.is_empty()
            && trimmed.len() <= 3
            && trimmed.chars().all(|c| c.is_ascii_digit())
        {
            drop_cap_indices.push(i);
        }
    }
    if !drop_cap_indices.is_empty() {
        for &dc_idx in drop_cap_indices.iter().rev() {
            let dc_line = reconstructed[dc_idx].clone();
            if let Some(target_idx) = (0..reconstructed.len()).find(|&j| {
                j != dc_idx
                    && !drop_cap_indices.contains(&j)
                    && reconstructed[j].avg_font_size_pt >= 13.0
                    && reconstructed[j].avg_font_size_pt < 45.0
                    && (reconstructed[j].bbox.y0 - dc_line.bbox.y0).abs() <= 42.0
                    && reconstructed[j].bbox.x0 >= dc_line.bbox.x0
            }) {
                let target = &mut reconstructed[target_idx];
                let mut prefix_word = dc_line.words[0].clone();
                prefix_word.text = format!("{}.", dc_line.plain_text().trim());
                prefix_word.style.font_size_pt = target.avg_font_size_pt;
                prefix_word.style.bold = true;
                prefix_word.baseline_y = target.baseline_y;
                let first_x0 = target
                    .words
                    .first()
                    .map(|w| w.bbox.x0)
                    .unwrap_or(dc_line.bbox.x0);
                prefix_word.bbox = RectPt::new(
                    (first_x0 - target.avg_font_size_pt * 1.4).max(dc_line.bbox.x0),
                    target.bbox.y0,
                    (first_x0 - 4.0).max(dc_line.bbox.x0 + 8.0),
                    target.bbox.y1,
                );
                target.words.insert(0, prefix_word);
                reconstructed.remove(dc_idx);
            } else {
                // Otherwise position by top y0 and cap font size so it never interrupts body paragraphs
                let dc = &mut reconstructed[dc_idx];
                dc.baseline_y = dc.bbox.y0 + 18.0;
                dc.bbox.y1 = dc.bbox.y0 + 22.0;
                dc.avg_font_size_pt = 20.0;
                for w in &mut dc.words {
                    w.style.font_size_pt = 20.0;
                    w.style.bold = true;
                }
            }
        }
    }

    // Sort reconstructed lines top-to-bottom
    reconstructed.sort_by(|a, b| {
        a.baseline_y
            .partial_cmp(&b.baseline_y)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // If the page had a 90°-rotated vertical margin reference stamp, append it cleanly at the bottom
    // so its characters are preserved without polluting horizontal body lines!
    if !rotated_glyphs.is_empty() {
        rotated_glyphs.sort_by(|a, b| {
            b.bbox
                .y0
                .partial_cmp(&a.bbox.y0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut rot_text = String::new();
        let mut prev_y: Option<f32> = None;
        let mut union_box = rotated_glyphs[0].bbox;
        for rg in &rotated_glyphs {
            union_box = union_box.union(&rg.bbox);
            if let Some(py) = prev_y {
                if (py - rg.bbox.y0).abs() > rg.font_size_pt * 0.85 && !rot_text.ends_with(' ') {
                    rot_text.push(' ');
                }
            }
            rot_text.push_str(&rg.unicode);
            prev_y = Some(rg.bbox.y0);
        }
        let rot_words: Vec<WordCluster> = rot_text
            .split_whitespace()
            .map(|tok| WordCluster {
                text: tok.to_string(),
                bbox: union_box,
                baseline_y: page.height_pt - 18.0,
                style: RunStyle {
                    font_family: "Helvetica Neue".to_string(),
                    font_size_pt: 8.5,
                    italic: true,
                    color: RgbaColor {
                        r: 95,
                        g: 99,
                        b: 104,
                        a: 255,
                    },
                    ..Default::default()
                },
            })
            .collect();
        if !rot_words.is_empty() {
            let last_y = reconstructed
                .iter()
                .rev()
                .find(|l| l.bbox.y0 < page.height_pt * 0.88)
                .map(|l| (l.bbox.y1 + 16.0).min(page.height_pt - 52.0))
                .unwrap_or(page.height_pt - 60.0);
            reconstructed.push(ReconstructedLine {
                words: rot_words,
                bbox: RectPt::new(56.7, last_y, 260.0, last_y + 10.0),
                baseline_y: last_y + 8.5,
                avg_font_size_pt: 8.5,
                has_vector_bullet: false,
            });
        }
    }

    reconstructed
}

fn cluster_line_glyphs_into_words(
    line_glyphs: &[GlyphIr],
    fonts: &[FontIr],
    annotations: &[AnnotIr],
    params: &Params,
    glued_fixes: &mut usize,
    tracked_fixes: &mut usize,
) -> Vec<WordCluster> {
    if line_glyphs.is_empty() {
        return Vec::new();
    }

    // Compute raw horizontal gaps between consecutive non-space glyphs
    let mut non_space_gaps = Vec::new();
    for pair in line_glyphs.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        if a.unicode.trim().is_empty() || b.unicode.trim().is_empty() {
            continue;
        }
        let tc = a.char_spacing_pt.max(0.0);
        let raw_gap = (b.bbox.x0 - a.bbox.x1 - tc).max(0.0);
        non_space_gaps.push(raw_gap);
    }

    let avg_size = line_glyphs.iter().map(|g| g.font_size_pt).sum::<f32>()
        / (line_glyphs.len() as f32);

    // Detect tracked heading ("F A C T U R E"):
    // All glyphs are single letters/digits, there are no explicit space glyphs, and gaps are uniform in [0.14*em .. 0.85*em]
    let has_explicit_space = line_glyphs.iter().any(|g| {
        g.unicode == " "
            || g.unicode == "\u{200B}"
            || g.actual_text.as_deref() == Some(" ")
            || g.actual_text.as_deref() == Some("\u{200B}")
    });
    let all_single_alnum = line_glyphs.len() >= 4
        && line_glyphs
            .iter()
            .all(|g| g.unicode.chars().count() == 1 && g.unicode.chars().all(|c| c.is_alphanumeric()));

    let mut is_tracked_word = false;
    let mut detected_tracking_pt = 0.0f32;

    if params.cluster_letter_spacing
        && !has_explicit_space
        && all_single_alnum
        && !non_space_gaps.is_empty()
    {
        let min_gap = non_space_gaps
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let max_gap = non_space_gaps.iter().copied().fold(0.0f32, f32::max);
        let mean_gap = non_space_gaps.iter().sum::<f32>() / (non_space_gaps.len() as f32);

        if min_gap >= avg_size * 0.14
            && max_gap <= avg_size * 0.95
            && (max_gap - min_gap) <= avg_size * 0.22
        {
            is_tracked_word = true;
            detected_tracking_pt = mean_gap;
            *tracked_fixes += 1;
        }
    }

    // Compute word-break gap threshold via 1D 2-means clustering when enough gaps exist.
    // When explicit space glyphs (' ') are already present on the line, word breaks are already
    // marked by those space glyphs, so only break non-space glyphs on wider column/missing-space jumps.
    let word_gap_threshold = if is_tracked_word {
        avg_size * 1.5
    } else {
        let base_thresh = compute_2means_word_gap_threshold(&non_space_gaps, avg_size);
        if has_explicit_space {
            base_thresh.max(avg_size * 0.26)
        } else {
            base_thresh
        }
    };

    let mut words: Vec<WordCluster> = Vec::new();
    let mut cur_text = String::new();
    let mut cur_bbox: Option<RectPt> = None;
    let mut cur_style: Option<RunStyle> = None;
    let mut cur_baseline_y = 0.0f32;
    let mut prev_x1: Option<f32> = None;
    let mut prev_tc = 0.0f32;

    for g in line_glyphs {
        let raw_str = if let Some(act) = &g.actual_text {
            if act == "\u{200B}" || act == " " {
                act.as_str()
            } else {
                g.unicode.as_str()
            }
        } else {
            g.unicode.as_str()
        };

        let norm = normalize_unicode_str(
            raw_str,
            params.profile,
            params.normalize_skia_zwsp,
            glued_fixes,
        );

        // Explicit space character ends current word
        if norm.trim().is_empty() {
            if !cur_text.is_empty() {
                if let (Some(bbox), Some(style)) = (cur_bbox.take(), cur_style.take()) {
                    words.push(WordCluster {
                        text: std::mem::take(&mut cur_text),
                        bbox,
                        baseline_y: cur_baseline_y,
                        style,
                    });
                }
            }
            prev_x1 = Some(g.bbox.x1);
            prev_tc = g.char_spacing_pt;
            continue;
        }

        let font_ir = fonts.get(g.font_idx).cloned().unwrap_or_default();
        let (mac_family, width_scale_pct) = map_pdf_font_to_mac(&font_ir, params.profile);

        // Check if any highlight or link annotation covers this glyph
        let mut highlight_color = None;
        let mut link_uri = None;
        for ann in annotations {
            if ann.bbox.intersection(&g.bbox).is_some() {
                match &ann.kind {
                    AnnotKind::Highlight => highlight_color = ann.color,
                    AnnotKind::Link { uri } => link_uri = Some(uri.clone()),
                    _ => {}
                }
            }
        }

        let g_style = RunStyle {
            font_family: mac_family,
            font_size_pt: (g.font_size_pt * 10.0).round() / 10.0,
            bold: font_ir.is_bold,
            italic: font_ir.is_italic,
            underline: false,
            strikethrough: false,
            color: g.color,
            highlight_color,
            char_tracking_pt: if is_tracked_word {
                detected_tracking_pt
            } else {
                0.0
            },
            width_scale_pct,
            link_uri,
        };

        let should_break_word = if let Some(px1) = prev_x1 {
            let effective_gap = (g.bbox.x0 - px1 - prev_tc.max(0.0)).max(0.0);
            effective_gap >= word_gap_threshold
        } else {
            false
        };

        if should_break_word && !cur_text.is_empty() {
            if let (Some(bbox), Some(style)) = (cur_bbox.take(), cur_style.take()) {
                words.push(WordCluster {
                    text: std::mem::take(&mut cur_text),
                    bbox,
                    baseline_y: cur_baseline_y,
                    style,
                });
            }
        }

        cur_text.push_str(&norm);
        cur_bbox = Some(match cur_bbox {
            Some(b) => b.union(&g.bbox),
            None => g.bbox,
        });
        if cur_style.is_none() {
            cur_style = Some(g_style);
            cur_baseline_y = g.baseline_y;
        }
        prev_x1 = Some(g.bbox.x1);
        prev_tc = g.char_spacing_pt;
    }

    if !cur_text.is_empty() {
        if let (Some(bbox), Some(style)) = (cur_bbox.take(), cur_style.take()) {
            words.push(WordCluster {
                text: cur_text,
                bbox,
                baseline_y: cur_baseline_y,
                style,
            });
        }
    }

    words
}

/// 1D 2-means clustering on positive inter-glyph gaps along a line.
/// Separates intra-word kerning gaps from inter-word space gaps.
pub(crate) fn compute_2means_word_gap_threshold(gaps: &[f32], avg_font_size: f32) -> f32 {
    let default_thresh = (avg_font_size * 0.18).max(1.6);
    if gaps.len() < 4 {
        return default_thresh;
    }
    let min_g = gaps.iter().copied().fold(f32::INFINITY, f32::min);
    let max_g = gaps.iter().copied().fold(0.0f32, f32::max);
    if max_g - min_g < avg_font_size * 0.12 {
        // All gaps on this line are uniform (single word or uniformly tracked/justified token without word spaces)
        if max_g <= avg_font_size * 0.45 {
            return max_g + avg_font_size * 0.12;
        }
        return default_thresh;
    }

    let mut c0 = min_g;
    let mut c1 = max_g.min(avg_font_size * 0.8);
    for _ in 0..8 {
        let mid = 0.5 * (c0 + c1);
        let mut sum0 = 0.0f32;
        let mut cnt0 = 0usize;
        let mut sum1 = 0.0f32;
        let mut cnt1 = 0usize;
        for &g in gaps {
            let clamped = g.min(avg_font_size * 1.2);
            if clamped <= mid {
                sum0 += clamped;
                cnt0 += 1;
            } else {
                sum1 += clamped;
                cnt1 += 1;
            }
        }
        if cnt0 > 0 {
            c0 = sum0 / (cnt0 as f32);
        }
        if cnt1 > 0 {
            c1 = sum1 / (cnt1 as f32);
        }
    }

    if c1 - c0 >= avg_font_size * 0.10 {
        (0.55 * c0 + 0.45 * c1).clamp(avg_font_size * 0.12, avg_font_size * 0.55)
    } else {
        default_thresh.clamp(avg_font_size * 0.12, avg_font_size * 0.30)
    }
}

/// Reconstruct a full `FlowDoc` from a `DocIr` and `Params`.
pub fn reconstruct_document(doc: &DocIr, params: &Params) -> (FlowDoc, ConvertReport) {
    let mut report = ConvertReport {
        page_count: doc.pages.len(),
        has_acroform: doc.has_acroform,
        form_field_count: doc.pages.iter().map(|p| p.form_fields.len()).sum(),
        ..Default::default()
    };

    let mut sections = Vec::with_capacity(doc.pages.len().max(1));
    let mut media_counter = 1usize;

    // Pre-build lines for all pages so we can detect repeating headers/footers across pages
    let mut page_lines: Vec<Vec<ReconstructedLine>> = Vec::with_capacity(doc.pages.len());
    for page in &doc.pages {
        let lines = build_lines_from_page(
            page,
            params,
            &mut report.glued_word_fixes,
            &mut report.tracked_heading_fixes,
        );
        page_lines.push(lines);
    }

    let (repeating_headers, repeating_footers) = detect_repeating_headers_footers(&doc.pages, &page_lines);

    for (page_idx, page) in doc.pages.iter().enumerate() {
        let detected_case = classify_page(page);
        let mut lines = std::mem::take(&mut page_lines[page_idx]);

        let word_count: usize = lines.iter().map(|l| l.words.len()).sum();
        report.total_words += word_count;

        let unmapped_count = page
            .glyphs
            .iter()
            .filter(|g| g.unicode.contains('\u{FFFD}'))
            .count();
        let unmapped_ratio = if page.glyphs.is_empty() {
            0.0
        } else {
            (unmapped_count as f32) / (page.glyphs.len() as f32)
        };

        let mut header_paragraphs = Vec::new();
        let mut footer_paragraphs = Vec::new();

        // Extract repeating header/footer lines on multi-page docs
        if doc.pages.len() >= 2 {
            lines.retain(|line| {
                let text = line.plain_text();
                if line.bbox.y1 <= page.height_pt * 0.10
                    && repeating_headers.iter().any(|h| strsim::jaro_winkler(h, &text) >= 0.85)
                {
                    header_paragraphs.push(line_to_paragraph(line, params, page.width_pt));
                    false
                } else if line.bbox.y0 >= page.height_pt * 0.90
                    && (repeating_footers.iter().any(|f| strsim::jaro_winkler(f, &text) >= 0.80)
                        || is_page_number_footer(&text))
                {
                    footer_paragraphs.push(line_to_paragraph(line, params, page.width_pt));
                    false
                } else {
                    true
                }
            });
        }

        // Convert FreeText annotations (Preview Fill & Sign) into lines so they are never lost!
        for ann in &page.annotations {
            if ann.kind == AnnotKind::FreeText {
                if let Some(txt) = &ann.text_contents {
                    let style = RunStyle {
                        font_family: "Helvetica Neue".to_string(),
                        font_size_pt: ann.font_size_pt.unwrap_or(11.0),
                        color: ann.color.unwrap_or(RgbaColor::BLACK),
                        ..Default::default()
                    };
                    lines.push(ReconstructedLine {
                        words: vec![WordCluster {
                            text: txt.clone(),
                            bbox: ann.bbox,
                            baseline_y: ann.bbox.y1 - 2.0,
                            style,
                        }],
                        bbox: ann.bbox,
                        baseline_y: ann.bbox.y1 - 2.0,
                        avg_font_size_pt: ann.font_size_pt.unwrap_or(11.0),
                        has_vector_bullet: false,
                    });
                }
            }
        }
        lines.sort_by(|a, b| {
            a.baseline_y
                .partial_cmp(&b.baseline_y)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 1. Detect Lattice Tables from horizontal/vertical ruling paths (supports multiple tables per page)
        let (mut tables, after_lattice_lines) = extract_lattice_tables(page, lines, params);

        // 2. Detect Stream Tables (borderless multi-column tables aligned by X coordinates, e.g. CPAM, Impôts)
        let (stream_tables, remaining_lines) =
            extract_stream_tables(&after_lattice_lines, params, page.width_pt);
        tables.extend(stream_tables);

        // Detect page margins from content bounding box (lines, tables, and non-background images)
        let (margin_left_pt, margin_right_pt, margin_top_pt, margin_bottom_pt) =
            estimate_page_margins(page, &remaining_lines, &tables);

        let content_width_pt = (page.width_pt - margin_left_pt - margin_right_pt).max(180.0);

        // Update right-indented table offsets relative to the actual `margin_left_pt` so their right edge aligns flush!
        for t in &mut tables {
            if t.source_bbox.x0 > margin_left_pt + 20.0 {
                t.left_indent_pt = (t.source_bbox.x0 - margin_left_pt).max(0.0);
            }
        }

        // Precompute full-width 1-cell stroked container frames so their top/bottom borders aren't emitted as standalone `<hr>` rules
        let frame_boxes = detect_single_cell_stroked_frames(page, content_width_pt);

        // Collect standalone full-width horizontal ruling lines not consumed by any lattice table or stroked frame
        let mut h_rule_paths: Vec<&PathIr> = page
            .paths
            .iter()
            .filter(|p| {
                let py = p.bbox.center_y();
                p.is_horizontal_line
                    && p.bbox.width() >= content_width_pt * 0.55
                    && !tables.iter().any(|t| {
                        py >= t.source_bbox.y0 - 4.0
                            && py <= t.source_bbox.y1 + 4.0
                            && p.bbox.x1 >= t.source_bbox.x0 - 4.0
                            && p.bbox.x0 <= t.source_bbox.x1 + 4.0
                    })
                    && !frame_boxes.iter().any(|(f_rect, _)| {
                        ((py - f_rect.y0).abs() <= 4.0 || (py - f_rect.y1).abs() <= 4.0)
                            && p.bbox.x1 >= f_rect.x0 - 4.0
                            && p.bbox.x0 <= f_rect.x1 + 4.0
                    })
            })
            .collect();
        h_rule_paths.sort_by(|a, b| {
            a.bbox
                .center_y()
                .partial_cmp(&b.bbox.center_y())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut dedup_h_rules: Vec<&PathIr> = Vec::new();
        for hr in h_rule_paths {
            if dedup_h_rules
                .last()
                .is_none_or(|prev| (hr.bbox.center_y() - prev.bbox.center_y()).abs() > 3.0)
            {
                dedup_h_rules.push(hr);
            }
        }
        let h_rule_ys: Vec<f32> = dedup_h_rules.iter().map(|p| p.bbox.center_y()).collect();

        // 3. Partition remaining lines into Full-Width bands, Top Side-by-Side Sender/Recipient layout tables,
        //    and 2-Column Flowing Body regions (reading Column 1 top-to-bottom then Column 2 top-to-bottom)
        let (zone_blocks, layout_table_count) = partition_lines_into_zone_blocks(
            page,
            &remaining_lines,
            &h_rule_ys,
            params,
            page.width_pt,
            page.height_pt,
            margin_left_pt,
            content_width_pt,
        );
        let table_count = tables.len() + layout_table_count;

        // Merge zone blocks, extracted tables, standalone horizontal rules, and images in vertical reading order
        let mut blocks_with_y: Vec<(f32, FlowBlock)> = zone_blocks;
        for t in tables {
            blocks_with_y.push((t.source_bbox.y0, FlowBlock::Table(t)));
        }
        for hr in dedup_h_rules {
            let color = hr
                .stroke_color
                .or(hr.fill_color)
                .unwrap_or(RgbaColor {
                    r: 140,
                    g: 140,
                    b: 145,
                    a: 255,
                });
            let rule_w = hr.bbox.width().clamp(content_width_pt * 0.55, content_width_pt);
            let hr_tbl = TableGrid {
                col_widths_pt: vec![rule_w],
                rows: vec![TableRow {
                    cells: vec![TableCell {
                        paragraphs: vec![Paragraph::default()],
                        grid_span: 1,
                        v_merge: None,
                        width_pt: rule_w,
                        shading_color: None,
                        border_top: None,
                        border_bottom: Some(CellBorder {
                            width_pt: hr.bbox.height().clamp(0.75, 1.75),
                            color,
                        }),
                        border_left: None,
                        border_right: None,
                    }],
                    min_height_pt: 1.0,
                    is_header_row: false,
                }],
                left_indent_pt: (hr.bbox.x0 - margin_left_pt).max(0.0),
                space_before_pt: 2.0,
                space_after_pt: 4.0,
                is_borderless_layout: true,
                source_bbox: hr.bbox,
            };
            blocks_with_y.push((hr.bbox.center_y(), FlowBlock::Table(hr_tbl)));
        }
        for img in &page.images {
            // Skip decorative full-bleed background textures on pages that already have digital text (including cover pages)
            if img.is_full_page_scan && page.glyphs.len() >= 15 {
                continue;
            }
            // If the page is a full-page scan and we already rendered an upright `page_raster_png`,
            // prefer `page_raster_png` so PDF transform rotations (e.g. 90° rotated JPEG scans) are upright!
            if img.is_full_page_scan && page.glyphs.len() < 15 && page.page_raster_png.is_some() {
                continue;
            }
            // Skip small watermark overlays that sit completely inside a larger image on the same page
            let is_nested_watermark = !img.is_full_page_scan
                && img.bbox.width() <= 140.0
                && img.bbox.height() <= 60.0
                && page.images.iter().any(|other| {
                    !std::ptr::eq(other, img)
                        && !other.is_full_page_scan
                        && other.bbox.width() >= img.bbox.width() * 2.5
                        && other.bbox.height() >= img.bbox.height() * 2.5
                        && img.bbox.x0 >= other.bbox.x0 - 2.0
                        && img.bbox.x1 <= other.bbox.x1 + 2.0
                        && img.bbox.y0 >= other.bbox.y0 - 2.0
                        && img.bbox.y1 <= other.bbox.y1 + 2.0
                });
            if is_nested_watermark {
                continue;
            }
            // Check if this image sits side-by-side with text lines (e.g. a photo on the left with text on its right, or a signature on the right of a letter)
            let has_side_by_side_text = !img.is_full_page_scan
                && img.bbox.width() <= page.width_pt * 0.68
                && remaining_lines.iter().any(|l| {
                    let y_overlap = l.bbox.y1.min(img.bbox.y1) - l.bbox.y0.max(img.bbox.y0);
                    let beside_x = l.bbox.x0 >= img.bbox.x1 - 12.0 || l.bbox.x1 <= img.bbox.x0 + 12.0;
                    y_overlap >= 8.0 && beside_x
                });
            let id = media_counter;
            media_counter += 1;
            blocks_with_y.push((
                img.bbox.y0,
                FlowBlock::Image(AnchoredImage {
                    media_id: id,
                    format: img.format.clone(),
                    data: img.data.clone(),
                    bbox_pt: img.bbox,
                    behind_doc: img.is_full_page_scan,
                    inline: !img.is_full_page_scan && !has_side_by_side_text,
                }),
            ));
        }

        blocks_with_y.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut blocks: Vec<FlowBlock> = blocks_with_y.into_iter().map(|(_, b)| b).collect();

        // Wrap paragraphs that sit inside a colored/dark filled vector banner or card
        // (e.g. DGFiP navy banner, Canva #8C52FF purple banner/card on pages 4 and 7 of a slide-style course,
        // light-gray section strip on its pages 4 and 5, or white card with colored offset shadow on its page 1)
        {
            // Tuple: (rect, shading_color, optional_shadow_border_color, force_white_text)
            let mut colored_banners: Vec<(RectPt, RgbaColor, Option<RgbaColor>, bool)> = Vec::new();
            for (p_idx, path) in page.paths.iter().enumerate() {
                if path.is_horizontal_line || path.is_vertical_line {
                    continue;
                }
                let Some(fill) = path.fill_color else {
                    continue;
                };
                let lum = (fill.r as u32) + (fill.g as u32) + (fill.b as u32);
                let max_c = fill.r.max(fill.g).max(fill.b);
                let min_c = fill.r.min(fill.g).min(fill.b);
                let saturation = max_c - min_c;
                let is_dark_or_saturated = (lum < 480 || (lum < 560 && saturation >= 80))
                    && path.bbox.width() >= 140.0
                    && path.bbox.height() >= 14.0
                    && path.bbox.height() <= 340.0;
                let is_light_section_strip = (620..=735).contains(&lum)
                    && saturation <= 20
                    && path.bbox.width() >= page.width_pt * 0.55
                    && path.bbox.height() >= 18.0
                    && path.bbox.height() <= 68.0;

                if is_dark_or_saturated {
                    // Check if a white card rectangle is drawn ON TOP of this colored rectangle (offset drop-shadow card)
                    let overlaid_white = page.paths[(p_idx + 1)..].iter().find(|later| {
                        !later.is_horizontal_line
                            && !later.is_vertical_line
                            && later
                                .fill_color
                                .is_some_and(|c| c.r >= 242 && c.g >= 242 && c.b >= 242)
                            && later.bbox.intersection(&path.bbox).is_some_and(|inter| {
                                inter.width() >= path.bbox.width() * 0.75
                                    && inter.height() >= path.bbox.height() * 0.75
                            })
                    });
                    if let Some(white_card) = overlaid_white {
                        colored_banners.push((
                            white_card.bbox.union(&path.bbox),
                            RgbaColor {
                                r: 255,
                                g: 255,
                                b: 255,
                                a: 255,
                            },
                            Some(fill),
                            false,
                        ));
                    } else {
                        colored_banners.push((path.bbox, fill, None, true));
                    }
                } else if is_light_section_strip {
                    colored_banners.push((path.bbox, fill, None, false));
                }
            }

            if !colored_banners.is_empty() {
                let mut new_blocks: Vec<FlowBlock> = Vec::with_capacity(blocks.len());
                let mut bi = 0;
                while bi < blocks.len() {
                    if let FlowBlock::Paragraph(p) = &blocks[bi] {
                        let p_cy = p.source_bbox.center_y();
                        let p_cx = p.source_bbox.center_x();
                        if let Some(&(b_rect, fill_color, shadow_bdr, force_white)) =
                            colored_banners.iter().find(|(r, _, _, _)| {
                                p_cy >= r.y0 - 4.0
                                    && p_cy <= r.y1 + 4.0
                                    && p_cx >= r.x0 - 8.0
                                    && p_cx <= r.x1 + 8.0
                            })
                        {
                            let mut banner_paras = Vec::new();
                            let mut union_bbox = b_rect.union(&p.source_bbox);
                            while bi < blocks.len() {
                                if let FlowBlock::Paragraph(cp) = &blocks[bi] {
                                    let cy = cp.source_bbox.center_y();
                                    let cx = cp.source_bbox.center_x();
                                    if cy >= b_rect.y0 - 4.0
                                        && cy <= b_rect.y1 + 4.0
                                        && cx >= b_rect.x0 - 8.0
                                        && cx <= b_rect.x1 + 8.0
                                    {
                                        let mut bp = cp.clone();
                                        bp.space_before_pt = 2.0;
                                        bp.space_after_pt = 2.0;
                                        bp.left_indent_pt = 0.0;
                                        if force_white {
                                            for run in &mut bp.runs {
                                                run.style.color = RgbaColor {
                                                    r: 255,
                                                    g: 255,
                                                    b: 255,
                                                    a: 255,
                                                };
                                                run.style.bold = true;
                                            }
                                        }
                                        union_bbox = union_bbox.union(&cp.source_bbox);
                                        banner_paras.push(bp);
                                        bi += 1;
                                        continue;
                                    }
                                }
                                break;
                            }
                            let box_w = b_rect.width().clamp(140.0, content_width_pt);
                            let box_indent = if b_rect.x0 > margin_left_pt + 20.0 {
                                (b_rect.x0 - margin_left_pt).min(content_width_pt - box_w).max(0.0)
                            } else {
                                0.0
                            };
                            let card_bdr = shadow_bdr.map(|color| CellBorder {
                                width_pt: 2.5,
                                color,
                            });
                            let banner_tbl = TableGrid {
                                col_widths_pt: vec![box_w],
                                rows: vec![TableRow {
                                    cells: vec![TableCell {
                                        paragraphs: banner_paras,
                                        grid_span: 1,
                                        v_merge: None,
                                        width_pt: box_w,
                                        shading_color: Some(fill_color),
                                        border_top: card_bdr,
                                        border_bottom: card_bdr,
                                        border_left: card_bdr,
                                        border_right: card_bdr,
                                    }],
                                    min_height_pt: b_rect.height().clamp(22.0, 120.0),
                                    is_header_row: false,
                                }],
                                left_indent_pt: box_indent,
                                space_before_pt: 0.0,
                                space_after_pt: 8.0,
                                is_borderless_layout: shadow_bdr.is_none(),
                                source_bbox: union_bbox,
                            };
                            new_blocks.push(FlowBlock::Table(banner_tbl));
                            continue;
                        }
                    }
                    new_blocks.push(blocks[bi].clone());
                    bi += 1;
                }
                blocks = new_blocks;
            }
        }

        // Wrap full-width 1-cell stroked container frames (e.g. a `Votre identité` frame on a registration certificate,
        // a `Détails bancaires` frame on an invoice) around the consecutive blocks inside them!
        {
            let frame_boxes = detect_single_cell_stroked_frames(page, content_width_pt);
            for (f_rect, stroke_color) in frame_boxes {
                let mut start_idx: Option<usize> = None;
                let mut end_idx: usize = 0;
                for (idx, b) in blocks.iter().enumerate() {
                    let bbox_opt = match b {
                        FlowBlock::Paragraph(p) => Some(p.source_bbox),
                        FlowBlock::Table(t) if t.is_borderless_layout => Some(t.source_bbox),
                        _ => None,
                    };
                    if let Some(bb) = bbox_opt {
                        if bb.center_y() >= f_rect.y0 - 3.0 && bb.center_y() <= f_rect.y1 + 3.0 {
                            if start_idx.is_none() {
                                start_idx = Some(idx);
                            }
                            end_idx = idx + 1;
                        } else if start_idx.is_some() && bb.y0 > f_rect.y1 + 3.0 {
                            break;
                        }
                    } else if start_idx.is_some() {
                        break;
                    }
                }

                if let Some(s) = start_idx {
                    if end_idx > s {
                        let mut cell_paras: Vec<Paragraph> = Vec::new();
                        for b in &blocks[s..end_idx] {
                            match b {
                                FlowBlock::Paragraph(p) => {
                                    let mut cp = p.clone();
                                    cp.left_indent_pt = (p.source_bbox.x0 - f_rect.x0).clamp(0.0, 220.0);
                                    cell_paras.push(cp);
                                }
                                FlowBlock::Table(t) => {
                                    // Convert 2-col borderless layout grid inside the frame into tabbed paragraphs
                                    if let Some(row) = t.rows.first() {
                                        if row.cells.len() == 2 {
                                            let left_ps = &row.cells[0].paragraphs;
                                            let right_ps = &row.cells[1].paragraphs;
                                            let max_len = left_ps.len().max(right_ps.len());
                                            let tab_pos = t.col_widths_pt.first().copied().unwrap_or(240.0);
                                            for k in 0..max_len {
                                                match (left_ps.get(k), right_ps.get(k)) {
                                                    (Some(lp), Some(rp)) if !rp.runs.is_empty() => {
                                                        let mut combined = lp.clone();
                                                        let left_ind =
                                                            (lp.source_bbox.x0 - f_rect.x0).clamp(0.0, 60.0);
                                                        combined.left_indent_pt = left_ind;
                                                        let tab_style = combined
                                                            .runs
                                                            .last()
                                                            .map(|r| r.style.clone())
                                                            .unwrap_or_default();
                                                        combined.runs.push(TextRun {
                                                            text: "\t".to_string(),
                                                            style: tab_style,
                                                        });
                                                        combined.runs.extend(rp.runs.clone());
                                                        combined.tab_stops = vec![TabStop {
                                                            pos_pt: left_ind + tab_pos,
                                                            align: TabAlignment::Left,
                                                            leader: TabLeader::None,
                                                        }];
                                                        cell_paras.push(combined);
                                                    }
                                                    (Some(lp), _) => {
                                                        let mut cloned = lp.clone();
                                                        cloned.left_indent_pt =
                                                            (lp.source_bbox.x0 - f_rect.x0).clamp(0.0, 60.0);
                                                        cell_paras.push(cloned);
                                                    }
                                                    (None, Some(rp)) => {
                                                        let mut indented = rp.clone();
                                                        indented.left_indent_pt =
                                                            (rp.source_bbox.x0 - f_rect.x0).clamp(0.0, 320.0);
                                                        cell_paras.push(indented);
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        if !cell_paras.is_empty() {
                            for (pi, cp) in cell_paras.iter_mut().enumerate() {
                                if pi == 0 {
                                    cp.space_before_pt = (cp.source_bbox.y0 - f_rect.y0 - 3.0).clamp(0.0, 10.0);
                                } else {
                                    cp.space_before_pt = cp.space_before_pt.clamp(0.0, 8.0);
                                }
                                cp.space_after_pt = 0.0;
                            }
                            let bdr = Some(CellBorder {
                                width_pt: 0.75,
                                color: stroke_color,
                            });
                            let frame_w = f_rect.width().clamp(content_width_pt * 0.70, content_width_pt);
                            let frame_tbl = TableGrid {
                                col_widths_pt: vec![frame_w],
                                rows: vec![TableRow {
                                    cells: vec![TableCell {
                                        paragraphs: cell_paras,
                                        grid_span: 1,
                                        v_merge: None,
                                        width_pt: frame_w,
                                        shading_color: None,
                                        border_top: bdr,
                                        border_bottom: bdr,
                                        border_left: bdr,
                                        border_right: bdr,
                                    }],
                                    min_height_pt: f_rect.height().clamp(36.0, 360.0),
                                    is_header_row: false,
                                }],
                                left_indent_pt: (f_rect.x0 - margin_left_pt).max(0.0),
                                space_before_pt: 0.0,
                                space_after_pt: 0.0,
                                is_borderless_layout: false,
                                source_bbox: f_rect,
                            };
                            blocks.splice(s..end_idx, std::iter::once(FlowBlock::Table(frame_tbl)));
                        }
                    }
                }
            }
        }

        // Propagate exact PDF vertical spacing (`space_before_pt`) between all consecutive blocks on the page
        let mut prev_bottom_y: Option<f32> = None;
        for block in &mut blocks {
            let curr_bbox = match block {
                FlowBlock::Paragraph(p) => Some(p.source_bbox),
                FlowBlock::Table(t) => Some(t.source_bbox),
                FlowBlock::Image(i) if i.inline => Some(i.bbox_pt),
                _ => None,
            };
            if let Some(cb) = curr_bbox {
                if let Some(prev_y1) = prev_bottom_y {
                    if cb.y0 >= prev_y1 - 2.0 {
                        let raw_gap = (cb.y0 - prev_y1 - 1.5).max(0.0);
                        let clamped_gap = match params.preset {
                            Preset::Fidele | Preset::Exact => raw_gap.clamp(0.0, 120.0),
                            Preset::Editable => raw_gap.clamp(0.0, 95.0),
                        };
                        match block {
                            FlowBlock::Paragraph(p) => {
                                p.space_before_pt = clamped_gap;
                                p.space_after_pt = 0.0;
                            }
                            FlowBlock::Table(t) => {
                                t.space_before_pt = clamped_gap;
                                t.space_after_pt = 0.0;
                            }
                            _ => {}
                        }
                    }
                } else {
                    let top_offset = if cb.y0 > margin_top_pt + 4.0 {
                        (cb.y0 - margin_top_pt).clamp(0.0, 220.0)
                    } else {
                        0.0
                    };
                    match block {
                        FlowBlock::Paragraph(p) => {
                            p.space_before_pt = top_offset;
                            p.space_after_pt = 0.0;
                        }
                        FlowBlock::Table(t) => {
                            t.space_before_pt = top_offset;
                            t.space_after_pt = 0.0;
                        }
                        _ => {}
                    }
                }
                prev_bottom_y = Some(cb.y1);
            }
        }

        // Fallback Rung 5: if page had no text and no images, attach page_raster_png if present
        let mut rung_used = FallbackRung::FullPipeline;
        if blocks.is_empty() {
            if let Some(png_bytes) = &page.page_raster_png {
                let id = media_counter;
                media_counter += 1;
                blocks.push(FlowBlock::Image(AnchoredImage {
                    media_id: id,
                    format: "png".to_string(),
                    data: png_bytes.clone(),
                    bbox_pt: RectPt::new(0.0, 0.0, page.width_pt, page.height_pt),
                    behind_doc: false,
                    inline: true,
                }));
                rung_used = FallbackRung::PageImagePlusText;
            }
        }

        sections.push(Section {
            page_width_pt: page.width_pt.max(200.0),
            page_height_pt: page.height_pt.max(200.0),
            margin_top_pt,
            margin_bottom_pt,
            margin_left_pt,
            margin_right_pt,
            column_count: 1,
            column_widths_pt: Vec::new(),
            column_space_pt: 18.0,
            header_paragraphs,
            footer_paragraphs,
            blocks,
        });

        report.pages.push(PageReport {
            page_index: page_idx,
            detected_case,
            rung_used,
            glyph_count: page.glyphs.len(),
            word_count,
            table_count,
            unmapped_glyph_ratio: unmapped_ratio,
            warnings: Vec::new(),
        });
    }

    // Cross-page continuation table linking in Editable mode
    if params.merge_continuation_tables && sections.len() >= 2 {
        merge_cross_page_tables(&mut sections);
    }

    let flow_doc = FlowDoc {
        title: doc.title.clone(),
        default_font_family: "Helvetica Neue".to_string(),
        default_font_size_pt: 11.0,
        sections,
        source_pages: doc.pages.clone(),
    };

    (flow_doc, report)
}

fn detect_repeating_headers_footers(
    pages: &[PageIr],
    page_lines: &[Vec<ReconstructedLine>],
) -> (Vec<String>, Vec<String>) {
    if pages.len() < 2 {
        return (Vec::new(), Vec::new());
    }
    let mut top_candidates = Vec::new();
    let mut bot_candidates = Vec::new();
    for (p, lines) in pages.iter().zip(page_lines.iter()) {
        for l in lines {
            if l.bbox.y1 <= p.height_pt * 0.10 {
                top_candidates.push(l.plain_text());
            } else if l.bbox.y0 >= p.height_pt * 0.90 {
                bot_candidates.push(l.plain_text());
            }
        }
    }
    let headers = find_repeated_strings(&top_candidates);
    let footers = find_repeated_strings(&bot_candidates);
    (headers, footers)
}

fn find_repeated_strings(candidates: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, a) in candidates.iter().enumerate() {
        if a.trim().len() < 3 {
            continue;
        }
        let matches = candidates
            .iter()
            .enumerate()
            .filter(|(j, b)| *j != i && strsim::jaro_winkler(a, b) >= 0.85)
            .count();
        if matches >= 1 && !out.iter().any(|existing: &String| strsim::jaro_winkler(existing, a) >= 0.85) {
            out.push(a.clone());
        }
    }
    out
}

fn is_page_number_footer(text: &str) -> bool {
    let trimmed = text.trim().to_ascii_lowercase();
    trimmed.starts_with("page ")
        || (trimmed.len() <= 12 && trimmed.chars().any(|c| c.is_ascii_digit()) && trimmed.contains('/'))
}

fn detect_single_cell_stroked_frames(page: &PageIr, content_width_pt: f32) -> Vec<(RectPt, RgbaColor)> {
    let mut frames = Vec::new();
    // 1. Explicit stroked axis-aligned rectangles spanning >= 70% of content width (e.g. the framed sections of a registration certificate)
    for p in &page.paths {
        if p.is_axis_aligned_rect
            && !p.is_horizontal_line
            && !p.is_vertical_line
            && let Some(sc) = p.stroke_color
            && p.bbox.width() >= content_width_pt * 0.70
            && p.bbox.width() <= page.width_pt * 0.96
            && p.bbox.height() >= 40.0
            && p.bbox.height() <= page.height_pt * 0.85
        {
            frames.push((p.bbox, sc));
        }
    }

    // 2. 4-sided boxes formed by 2 horizontal + 2 vertical stroked lines (e.g. a `Détails bancaires` box on an invoice)
    let stroked_lines: Vec<&PathIr> = page
        .paths
        .iter()
        .filter(|p| (p.is_horizontal_line || p.is_vertical_line) && p.stroke_color.is_some())
        .collect();
    if !stroked_lines.is_empty() {
        let mut sorted = stroked_lines;
        sorted.sort_by(|a, b| {
            a.bbox
                .y0
                .partial_cmp(&b.bbox.y0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut clusters: Vec<(RectPt, Vec<&PathIr>)> = Vec::new();
        for p in sorted {
            if let Some((cbox, list)) = clusters.last_mut() {
                if p.bbox.y0 <= cbox.y1 + 3.5 {
                    *cbox = cbox.union(&p.bbox);
                    list.push(p);
                    continue;
                }
            }
            clusters.push((p.bbox, vec![p]));
        }
        for (cbox, list) in clusters {
            if cbox.width() < content_width_pt * 0.70 || cbox.height() < 40.0 {
                continue;
            }
            let mut hy: Vec<f32> = list
                .iter()
                .filter(|p| p.is_horizontal_line)
                .map(|p| p.bbox.center_y())
                .collect();
            let mut vx: Vec<f32> = list
                .iter()
                .filter(|p| p.is_vertical_line)
                .map(|p| p.bbox.center_x())
                .collect();
            let gy = cluster_1d_coords(&mut hy, 4.0);
            let gx = cluster_1d_coords(&mut vx, 4.0);
            if gy.len() == 2 && gx.len() == 2 {
                let sc = list
                    .iter()
                    .find_map(|p| p.stroke_color)
                    .unwrap_or(RgbaColor::BLACK);
                frames.push((RectPt::new(gx[0], gy[0], gx[1], gy[1]), sc));
            }
        }
    }

    frames
}

fn extract_lattice_tables(
    page: &PageIr,
    lines: Vec<ReconstructedLine>,
    params: &Params,
) -> (Vec<TableGrid>, Vec<ReconstructedLine>) {
    // Group ruling paths into vertically connected clusters so a page with multiple tables
    // (or a top banner rect + 2 separate tables) extracts each table independently!
    let mut candidate_paths: Vec<&PathIr> = page
        .paths
        .iter()
        .filter(|p| {
            // Ignore unstroked white/near-white background container rectangles (e.g. HTML-to-PDF white <div> backgrounds)
            let is_unstroked_white_rect = p.stroke_color.is_none()
                && p.fill_color
                    .is_some_and(|c| c.r >= 245 && c.g >= 245 && c.b >= 245);
            if is_unstroked_white_rect && !p.is_horizontal_line && !p.is_vertical_line {
                return false;
            }
            // Ignore large unstroked decorative background rectangles (e.g. Canva slide blocks in a slide-style course)
            if p.stroke_color.is_none()
                && !p.is_horizontal_line
                && !p.is_vertical_line
                && p.bbox.height() > 45.0
            {
                return false;
            }
            p.is_horizontal_line
                || p.is_vertical_line
                || (p.is_axis_aligned_rect
                    && p.bbox.width() >= 25.0
                    && p.bbox.height() >= 10.0
                    && p.bbox.area() <= page.width_pt * page.height_pt * 0.85)
        })
        .collect();

    if candidate_paths.is_empty() {
        return (Vec::new(), lines);
    }

    candidate_paths.sort_by(|a, b| {
        a.bbox
            .y0
            .partial_cmp(&b.bbox.y0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut path_clusters: Vec<(RectPt, Vec<&PathIr>)> = Vec::new();
    for p in candidate_paths {
        if let Some((cluster_box, list)) = path_clusters.last_mut() {
            if p.bbox.y0 <= cluster_box.y1 + 3.5 {
                *cluster_box = cluster_box.union(&p.bbox);
                list.push(p);
                continue;
            }
        }
        path_clusters.push((p.bbox, vec![p]));
    }

    let mut extracted_tables = Vec::new();
    let mut current_lines = lines;

    for (t_bbox, cluster) in path_clusters {
        // Require at least one explicit line or stroked rectangle in the cluster (never build a lattice table from unstroked blocks alone)
        let has_explicit_rule_or_stroke = cluster
            .iter()
            .any(|p| p.is_horizontal_line || p.is_vertical_line || p.stroke_color.is_some());
        if !has_explicit_rule_or_stroke {
            continue;
        }

        let mut h_coords_y: Vec<f32> = Vec::new();
        let mut v_coords_x: Vec<f32> = Vec::new();

        for p in &cluster {
            if p.is_horizontal_line {
                h_coords_y.push(p.bbox.center_y());
            } else if p.is_vertical_line {
                v_coords_x.push(p.bbox.center_x());
                h_coords_y.push(p.bbox.y0);
                h_coords_y.push(p.bbox.y1);
            } else if p.is_axis_aligned_rect {
                h_coords_y.push(p.bbox.y0);
                h_coords_y.push(p.bbox.y1);
                // Add left/right of a rect as vertical table grid lines unless it's a single full-width banner box
                if p.stroke_color.is_some() || p.bbox.width() < page.width_pt * 0.72 {
                    v_coords_x.push(p.bbox.x0);
                    v_coords_x.push(p.bbox.x1);
                }
            }
        }

        let grid_y = cluster_1d_coords(&mut h_coords_y, 4.0);
        let grid_x = cluster_1d_coords(&mut v_coords_x, 4.0);

        if grid_y.len() < 2 || grid_x.len() < 2 {
            continue;
        }

        let num_rows = grid_y.len() - 1;
        let num_cols = grid_x.len() - 1;

        // Allow:
        // - Standard tables (`num_rows >= 2 && num_cols >= 2`)
        // - 1-row multi-column tables (`num_rows == 1 && num_cols >= 2`, e.g. a `Total | 99,00 €` row on an invoice)
        // - Multi-row 1-column header boxes (`num_rows >= 2 && num_cols == 1 && t_bbox.width() <= page.width_pt * 0.65`,
        //   e.g. the sender box of a quote and the `FACTURE N° / Date` box of an invoice)
        let valid_dims = (num_rows >= 2 && num_cols >= 2)
            || (num_rows == 1 && num_cols >= 2)
            || (num_rows >= 2 && num_cols == 1 && t_bbox.width() <= page.width_pt * 0.65);
        if !valid_dims {
            continue;
        }

        let mut col_widths_pt: Vec<f32> = grid_x.windows(2).map(|w| (w[1] - w[0]).max(18.0)).collect();

        // Sample actual border color from the cluster's stroked/ruling paths
        let sampled_border_color = cluster
            .iter()
            .find_map(|p| {
                p.stroke_color.or_else(|| {
                    if p.is_horizontal_line || p.is_vertical_line {
                        p.fill_color
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(RgbaColor {
                r: 160,
                g: 160,
                b: 160,
                a: 255,
            });

        let border = Some(CellBorder {
            width_pt: 0.75,
            color: sampled_border_color,
        });

        // Check if the PDF has explicit non-white cell background rectangles inside this table
        let has_pdf_cell_fills = page.paths.iter().any(|p| {
            !p.is_horizontal_line
                && !p.is_vertical_line
                && p.bbox.intersection(&t_bbox).is_some()
                && p.bbox.width() <= page.width_pt * 0.90
                && p.bbox.height() <= 80.0
                && p.fill_color
                    .is_some_and(|c| !(c.r >= 248 && c.g >= 248 && c.b >= 248))
        });
        let has_dark_borders = sampled_border_color.r < 90
            && sampled_border_color.g < 90
            && sampled_border_color.b < 90;
        let is_right_totals_box = num_cols == 2 && grid_x[0] > page.width_pt * 0.45;
        let has_h_border_at = |y: f32| -> bool {
            cluster.iter().any(|p| {
                if p.is_horizontal_line && p.stroke_color.is_some() {
                    (p.bbox.center_y() - y).abs() <= 4.0
                } else if p.is_axis_aligned_rect
                    && (p.stroke_color.is_some() || p.fill_color.is_some())
                {
                    (p.bbox.y0 - y).abs() <= 4.0 || (p.bbox.y1 - y).abs() <= 4.0
                } else {
                    false
                }
            })
        };

        let mut rows: Vec<TableRow> = (0..num_rows)
            .map(|r| {
                let row_h = (grid_y[r + 1] - grid_y[r]).max(14.0);
                let row_bdr_top = if r == 0 || has_h_border_at(grid_y[r]) {
                    border
                } else {
                    None
                };
                let row_bdr_bot = if r + 1 == num_rows || has_h_border_at(grid_y[r + 1]) {
                    border
                } else {
                    None
                };
                let cells = (0..num_cols)
                    .map(|c| {
                        let cell_cx = 0.5 * (grid_x[c] + grid_x[c + 1]);
                        let cell_cy = 0.5 * (grid_y[r] + grid_y[r + 1]);
                        let pdf_cell_fill = page.paths.iter().find_map(|p| {
                            if p.is_horizontal_line || p.is_vertical_line {
                                return None;
                            }
                            let fc = p.fill_color?;
                            if fc.r >= 248 && fc.g >= 248 && fc.b >= 248 {
                                return None;
                            }
                            if p.bbox.width() <= page.width_pt * 0.90
                                && p.bbox.height() <= (row_h * 3.5).max(80.0)
                                && cell_cx >= p.bbox.x0 - 2.0
                                && cell_cx <= p.bbox.x1 + 2.0
                                && cell_cy >= p.bbox.y0 - 2.0
                                && cell_cy <= p.bbox.y1 + 2.0
                            {
                                Some(fc)
                            } else {
                                None
                            }
                        });
                        let shading_color = if pdf_cell_fill.is_some() {
                            pdf_cell_fill
                        } else if has_pdf_cell_fills || has_dark_borders || is_right_totals_box {
                            None
                        } else if r == 0 && num_rows >= 2 && num_cols >= 2 {
                            Some(RgbaColor {
                                r: 235,
                                g: 241,
                                b: 250,
                                a: 255,
                            })
                        } else {
                            None
                        };
                        TableCell {
                            paragraphs: Vec::new(),
                            grid_span: 1,
                            v_merge: None,
                            width_pt: col_widths_pt[c],
                            shading_color,
                            border_top: row_bdr_top,
                            border_bottom: row_bdr_bot,
                            border_left: border,
                            border_right: border,
                        }
                    })
                    .collect();
                TableRow {
                    cells,
                    min_height_pt: row_h,
                    is_header_row: r == 0 && num_rows >= 2 && num_cols >= 2 && !is_right_totals_box,
                }
            })
            .collect();

        let mut next_remaining = Vec::new();
        let mut consumed_lines = Vec::new();
        let expanded_t_bbox = RectPt::new(
            t_bbox.x0 - 4.0,
            t_bbox.y0 - 4.0,
            t_bbox.x1 + 4.0,
            t_bbox.y1 + 4.0,
        );

        for line in current_lines {
            let cy = line.bbox.center_y();
            let row_idx =
                (0..num_rows).find(|&r| cy >= grid_y[r] - 3.5 && cy <= grid_y[r + 1] + 3.5);

            if line.avg_font_size_pt <= 18.0
                && expanded_t_bbox.intersection(&line.bbox).is_some()
                && let Some(r) = row_idx
            {
                consumed_lines.push(line.clone());
                let mut col_words: Vec<Vec<WordCluster>> = vec![Vec::new(); num_cols];
                for w in &line.words {
                    let wx = w.bbox.center_x();
                    let c = (0..num_cols)
                        .find(|&c| wx >= grid_x[c] - 4.0 && wx <= grid_x[c + 1] + 4.0)
                        .unwrap_or_else(|| {
                            if wx < grid_x[0] {
                                0
                            } else {
                                num_cols - 1
                            }
                        });
                    col_words[c].push(w.clone());
                }

                for (c, words_in_cell) in col_words.into_iter().enumerate() {
                    if words_in_cell.is_empty() {
                        continue;
                    }
                    let mut cell_bbox = words_in_cell[0].bbox;
                    for w in &words_in_cell[1..] {
                        cell_bbox = cell_bbox.union(&w.bbox);
                    }
                    let cell_line = ReconstructedLine {
                        words: words_in_cell,
                        bbox: cell_bbox,
                        baseline_y: line.baseline_y,
                        avg_font_size_pt: line.avg_font_size_pt,
                        has_vector_bullet: false,
                    };
                    let mut p = line_to_paragraph(&cell_line, params, col_widths_pt[c]);
                    let cell_mid_x = 0.5 * (grid_x[c] + grid_x[c + 1]);
                    let left_pad = (cell_line.bbox.x0 - grid_x[c]).max(0.0);
                    let right_pad = (grid_x[c + 1] - cell_line.bbox.x1).max(0.0);
                    if num_cols == 1 && (cell_line.bbox.center_x() - cell_mid_x).abs() <= 18.0 {
                        p.alignment = Alignment::Center;
                    } else if (is_numeric_or_currency_cell(&cell_line.plain_text()) && c > 0)
                        || (is_right_totals_box && left_pad > right_pad + 8.0)
                    {
                        p.alignment = Alignment::Right;
                    } else {
                        p.alignment = Alignment::Left;
                    }
                    p.space_before_pt = 0.0;
                    p.space_after_pt = 0.0;
                    rows[r].cells[c].paragraphs.push(p);
                }
            } else {
                next_remaining.push(line);
            }
        }

        let populated_cells: usize = rows
            .iter()
            .flat_map(|r| r.cells.iter())
            .filter(|c| !c.paragraphs.is_empty())
            .count();

        if populated_cells < 2 {
            // Restore consumed lines if candidate table had fewer than 2 populated cells
            next_remaining.extend(consumed_lines);
            next_remaining.sort_by(|a, b| {
                a.baseline_y
                    .partial_cmp(&b.baseline_y)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            current_lines = next_remaining;
            continue;
        }

        current_lines = next_remaining;

        // If any single-line cell in column `c` nearly fills or slightly overflows `col_widths_pt[c]`
        // (e.g. clipped cell in Google Sheets / Excel), borrow a few points from the slackest columns
        // so cell padding in HTML/Word never causes a single-line PDF cell to wrap onto 2 lines!
        if num_cols >= 2 {
            let mut max_line_w = vec![0.0f32; num_cols];
            for row in &rows {
                for (c, cell) in row.cells.iter().enumerate() {
                    if cell.paragraphs.len() == 1 {
                        let w = cell.paragraphs[0].source_bbox.width();
                        if w > max_line_w[c] {
                            max_line_w[c] = w;
                        }
                    }
                }
            }
            for c in 0..num_cols {
                let needed = max_line_w[c] + 10.0 - col_widths_pt[c];
                if needed > 0.5 && needed <= 24.0 {
                    // Find column with maximum slack
                    if let Some((donor, slack)) = (0..num_cols)
                        .filter(|&k| k != c)
                        .map(|k| (k, col_widths_pt[k] - (max_line_w[k] + 12.0)))
                        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    {
                        let transfer = needed.min(slack.max(0.0));
                        if transfer > 0.5 {
                            col_widths_pt[c] += transfer;
                            col_widths_pt[donor] -= transfer;
                        }
                    }
                }
            }
        }

        for row in &mut rows {
            for (c, cell) in row.cells.iter_mut().enumerate() {
                cell.width_pt = col_widths_pt[c];
                if cell.paragraphs.is_empty() {
                    cell.paragraphs.push(Paragraph::default());
                }
            }
        }

        let left_indent_pt = if grid_x[0] > page.width_pt * 0.40 {
            (grid_x[0] - 36.0).max(0.0)
        } else {
            0.0
        };

        extracted_tables.push(TableGrid {
            col_widths_pt,
            rows,
            left_indent_pt,
            space_before_pt: 6.0,
            space_after_pt: 6.0,
            is_borderless_layout: false,
            source_bbox: t_bbox,
        });
    }

    (extracted_tables, current_lines)
}

/// Detect Stream Tables (borderless multi-column tables aligned by X coordinates without vertical rules,
/// such as CPAM/Mutuelle statements, DGFiP tax breakdown tables, and modern invoices).
fn extract_stream_tables(
    lines: &[ReconstructedLine],
    params: &Params,
    page_width_pt: f32,
) -> (Vec<TableGrid>, Vec<ReconstructedLine>) {
    if lines.len() < 3 {
        return (Vec::new(), lines.to_vec());
    }

    // Split each line into horizontal column segments where adjacent words have gap >= max(15.5 pt, 1.35 * font_size)
    let line_segments: Vec<Vec<ReconstructedLine>> = lines
        .iter()
        .map(|l| {
            if l.avg_font_size_pt > 14.0 {
                vec![l.clone()]
            } else {
                split_line_by_horizontal_gaps(l, (l.avg_font_size_pt * 1.35).max(15.5))
            }
        })
        .collect();

    let mut tables = Vec::new();
    let mut remaining = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Look for a contiguous run of >= 3 lines with >= 3 segments each and vertical step <= 48 pt
        if line_segments[i].len() >= 3 {
            let mut j = i + 1;
            while j < lines.len() {
                let step = lines[j].baseline_y - lines[j - 1].baseline_y;
                if step > 0.0 && step <= 48.0 && line_segments[j].len() >= 3 {
                    j += 1;
                } else {
                    break;
                }
            }

            if j - i >= 3 {
                // Build a stream table from lines[i..j]
                let slice_segs = &line_segments[i..j];
                let num_cols = slice_segs
                    .iter()
                    .map(|s| s.len())
                    .max()
                    .unwrap_or(3)
                    .clamp(3, 7);

                // Collect X start positions for each column index
                let mut col_starts: Vec<f32> = Vec::new();
                for segs in slice_segs {
                    if segs.len() == num_cols {
                        for (c, seg) in segs.iter().enumerate() {
                            if col_starts.len() <= c {
                                col_starts.push(seg.bbox.x0);
                            } else {
                                col_starts[c] = col_starts[c].min(seg.bbox.x0);
                            }
                        }
                    }
                }
                if col_starts.len() < num_cols {
                    col_starts = (0..num_cols)
                        .map(|c| 56.7 + (c as f32) * (480.0 / (num_cols as f32)))
                        .collect();
                }

                let mut t_bbox = lines[i].bbox;
                for l in &lines[i + 1..j] {
                    t_bbox = t_bbox.union(&l.bbox);
                }

                let mut col_widths_pt = Vec::with_capacity(num_cols);
                for c in 0..num_cols {
                    let x0 = col_starts[c];
                    let x1 = if c + 1 < num_cols {
                        col_starts[c + 1]
                    } else {
                        t_bbox.x1.max(page_width_pt - 56.7)
                    };
                    col_widths_pt.push((x1 - x0).max(36.0));
                }

                let subtle_border = Some(CellBorder {
                    width_pt: 0.5,
                    color: RgbaColor {
                        r: 195,
                        g: 200,
                        b: 208,
                        a: 255,
                    },
                });

                let mut rows = Vec::with_capacity(j - i);
                for (r_offset, segs) in slice_segs.iter().enumerate() {
                    let is_header = r_offset == 0;
                    let mut cells: Vec<TableCell> = (0..num_cols)
                        .map(|c| TableCell {
                            paragraphs: Vec::new(),
                            grid_span: 1,
                            v_merge: None,
                            width_pt: col_widths_pt[c],
                            shading_color: if is_header {
                                Some(RgbaColor {
                                    r: 235,
                                    g: 241,
                                    b: 250,
                                    a: 255,
                                })
                            } else {
                                None
                            },
                            border_top: if is_header { subtle_border } else { None },
                            border_bottom: subtle_border,
                            border_left: None,
                            border_right: None,
                        })
                        .collect();

                    for seg in segs {
                        let cx = seg.bbox.center_x();
                        let col_idx = (0..num_cols)
                            .rev()
                            .find(|&c| cx >= col_starts[c] - 10.0)
                            .unwrap_or(0);
                        let mut p = line_to_paragraph(seg, params, col_widths_pt[col_idx]);
                        if is_numeric_or_currency_cell(&seg.plain_text()) && col_idx > 0 {
                            p.alignment = Alignment::Right;
                        } else {
                            p.alignment = Alignment::Left;
                        }
                        p.space_before_pt = 1.5;
                        p.space_after_pt = 1.5;
                        cells[col_idx].paragraphs.push(p);
                    }

                    for cell in &mut cells {
                        if cell.paragraphs.is_empty() {
                            cell.paragraphs.push(Paragraph::default());
                        }
                    }

                    rows.push(TableRow {
                        cells,
                        min_height_pt: 18.0,
                        is_header_row: is_header,
                    });
                }

                tables.push(TableGrid {
                    col_widths_pt,
                    rows,
                    left_indent_pt: 0.0,
                    space_before_pt: 6.0,
                    space_after_pt: 6.0,
                    is_borderless_layout: false,
                    source_bbox: t_bbox,
                });

                i = j;
                continue;
            }
        }

        remaining.push(lines[i].clone());
        i += 1;
    }

    (tables, remaining)
}

fn split_line_by_horizontal_gaps(line: &ReconstructedLine, min_gap_pt: f32) -> Vec<ReconstructedLine> {
    if line.words.is_empty() {
        return Vec::new();
    }
    let mut segments = Vec::new();
    let mut cur_words = vec![line.words[0].clone()];

    for w in &line.words[1..] {
        let prev = cur_words.last().unwrap();
        let gap = w.bbox.x0 - prev.bbox.x1;
        if gap >= min_gap_pt {
            segments.push(build_subline(cur_words, line.baseline_y, line.avg_font_size_pt));
            cur_words = vec![w.clone()];
        } else {
            cur_words.push(w.clone());
        }
    }
    if !cur_words.is_empty() {
        segments.push(build_subline(cur_words, line.baseline_y, line.avg_font_size_pt));
    }
    segments
}

fn build_subline(words: Vec<WordCluster>, baseline_y: f32, avg_font_size_pt: f32) -> ReconstructedLine {
    let mut bbox = words[0].bbox;
    for w in &words[1..] {
        bbox = bbox.union(&w.bbox);
    }
    ReconstructedLine {
        words,
        bbox,
        baseline_y,
        avg_font_size_pt,
        has_vector_bullet: false,
    }
}

fn is_numeric_or_currency_cell(text: &str) -> bool {
    let t = text.trim();
    t.ends_with('€')
        || t.ends_with('%')
        || (t.chars().any(|c| c.is_ascii_digit())
            && t.chars().all(|c| {
                c.is_ascii_digit()
                    || matches!(c, ' ' | '\u{00A0}' | '\u{202F}' | ',' | '.' | '-' | '+' | '€' | '%' | '/')
            }))
}

fn detect_half_box_style(page: &PageIr, lines: &[ReconstructedLine]) -> (Option<RgbaColor>, Option<CellBorder>) {
    if lines.is_empty() {
        return (None, None);
    }
    let mut bbox = lines[0].bbox;
    for l in &lines[1..] {
        bbox = bbox.union(&l.bbox);
    }
    let cx = bbox.center_x();
    let cy = bbox.center_y();

    // 1. Check for a shaded box covering these lines (e.g. a light grey `#ECEFF1` totals box on an invoice)
    let shading = page.paths.iter().find_map(|p| {
        if p.is_horizontal_line || p.is_vertical_line {
            return None;
        }
        let fc = p.fill_color?;
        if fc.r >= 248 && fc.g >= 248 && fc.b >= 248 {
            return None;
        }
        if p.bbox.width() <= page.width_pt * 0.65
            && p.bbox.height() >= 18.0
            && p.bbox.height() <= 160.0
            && cx >= p.bbox.x0 - 4.0
            && cx <= p.bbox.x1 + 4.0
            && cy >= p.bbox.y0 - 4.0
            && cy <= p.bbox.y1 + 4.0
        {
            Some(fc)
        } else {
            None
        }
    });

    // 2. Check for a 4-sided stroked box covering these lines (e.g. the recipient box of an invoice)
    let has_top = page.paths.iter().any(|p| {
        p.is_horizontal_line
            && p.stroke_color.is_some()
            && p.bbox.width() <= page.width_pt * 0.65
            && p.bbox.center_y() <= bbox.y0 + 4.0
            && p.bbox.center_y() >= bbox.y0 - 42.0
            && cx >= p.bbox.x0 - 4.0
            && cx <= p.bbox.x1 + 4.0
    });
    let has_bot = page.paths.iter().any(|p| {
        p.is_horizontal_line
            && p.stroke_color.is_some()
            && p.bbox.width() <= page.width_pt * 0.65
            && p.bbox.center_y() >= bbox.y1 - 4.0
            && p.bbox.center_y() <= bbox.y1 + 48.0
            && cx >= p.bbox.x0 - 4.0
            && cx <= p.bbox.x1 + 4.0
    });
    let has_left = page.paths.iter().any(|p| {
        p.is_vertical_line
            && p.stroke_color.is_some()
            && p.bbox.center_x() <= bbox.x0 + 4.0
            && p.bbox.center_x() >= bbox.x0 - 32.0
            && cy >= p.bbox.y0 - 4.0
            && cy <= p.bbox.y1 + 4.0
    });
    let has_right = page.paths.iter().any(|p| {
        p.is_vertical_line
            && p.stroke_color.is_some()
            && p.bbox.center_x() >= bbox.x1 - 4.0
            && p.bbox.center_x() <= bbox.x1 + 190.0
            && cy >= p.bbox.y0 - 4.0
            && cy <= p.bbox.y1 + 4.0
    });
    let border = if has_top && has_bot && has_left && has_right {
        Some(CellBorder {
            width_pt: 0.75,
            color: RgbaColor::BLACK,
        })
    } else {
        None
    };

    (shading, border)
}

fn line_to_col_paragraph(line: &ReconstructedLine, params: &Params, col_w: f32) -> Paragraph {
    // Check if this column line has an internal label + right-aligned amount gap (e.g. "Total HT   1 234,00 €" or "Acompte   500,00 € HT")
    if line.words.len() >= 2 {
        let mut best_split: Option<(usize, f32)> = None;
        for idx in 0..(line.words.len() - 1) {
            let gap = line.words[idx + 1].bbox.x0 - line.words[idx].bbox.x1;
            if gap >= 12.0 {
                if best_split.is_none_or(|(_, bg)| gap > bg) {
                    best_split = Some((idx, gap));
                }
            }
        }
        if let Some((split_idx, _)) = best_split {
            let right_words = &line.words[split_idx + 1..];
            let right_txt: String = right_words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let is_amt = is_numeric_or_currency_cell(&right_txt)
                || right_txt.to_ascii_lowercase().contains("euro");
            if is_amt {
                let left_words = &line.words[..=split_idx];
                let mut runs = words_to_runs(left_words);
                let tab_style = runs.last().map(|r| r.style.clone()).unwrap_or_default();
                runs.push(TextRun {
                    text: "\t".to_string(),
                    style: tab_style,
                });
                runs.extend(words_to_runs(right_words));
                return Paragraph {
                    runs,
                    alignment: Alignment::Left,
                    space_before_pt: 0.0,
                    space_after_pt: 0.5,
                    left_indent_pt: 0.0,
                    right_indent_pt: 0.0,
                    first_line_indent_pt: 0.0,
                    line_spacing: compute_line_spacing(
                        line.avg_font_size_pt,
                        line.avg_font_size_pt * 1.18,
                        params,
                    ),
                    heading_level: None,
                    list_kind: None,
                    tab_stops: vec![TabStop {
                        pos_pt: col_w,
                        align: TabAlignment::Right,
                        leader: TabLeader::None,
                    }],
                    page_break_before: false,
                    keep_with_next: false,
                    auto_hyphenation: false,
                    source_bbox: line.bbox,
                };
            }
        }
    }
    let mut p = line_to_paragraph(line, params, col_w);
    p.alignment = Alignment::Left;
    p.left_indent_pt = 0.0;
    p.space_before_pt = 0.0;
    p.space_after_pt = 0.5;
    p
}

/// Partition lines into:
/// - Full-width flowing paragraphs,
/// - Top side-by-side Sender/Recipient blocks (emitted as a 2-column borderless `TableGrid`),
/// - 2-Column Flowing Body regions (read Column 1 top-to-bottom, then Column 2 top-to-bottom).
fn partition_lines_into_zone_blocks(
    page: &PageIr,
    lines: &[ReconstructedLine],
    h_rule_ys: &[f32],
    params: &Params,
    page_width_pt: f32,
    page_height_pt: f32,
    margin_left_pt: f32,
    content_width_pt: f32,
) -> (Vec<(f32, FlowBlock)>, usize) {
    if lines.is_empty() {
        return (Vec::new(), 0);
    }

    let page_mid = page_width_pt * 0.50;

    // First pass: split any line that has a central column gutter gap (>= 28 pt crossing near page_mid)
    // unless the right segment is a short currency/total amount (which belongs to `try_build_tabbed_pair_paragraph`).
    let mut split_lines: Vec<ReconstructedLine> = Vec::with_capacity(lines.len() * 2);
    for l in lines {
        if l.avg_font_size_pt >= 15.0 {
            split_lines.push(l.clone());
            continue;
        }
        let mut best_split_idx: Option<usize> = None;
        let mut best_gap = 0.0f32;
        for i in 1..l.words.len() {
            let left_x0 = l.words[i - 1].bbox.x0;
            let left_x1 = l.words[i - 1].bbox.x1;
            let right_x0 = l.words[i].bbox.x0;
            let gap = right_x0 - left_x1;
            if gap >= 28.0
                && left_x0 < page_mid - 10.0
                && left_x1 <= page_mid + 12.0
                && right_x0 >= page_mid - 25.0
                && gap > best_gap
            {
                best_gap = gap;
                best_split_idx = Some(i);
            }
        }
        if let Some(split_idx) = best_split_idx {
            let left = build_subline(l.words[..split_idx].to_vec(), l.baseline_y, l.avg_font_size_pt);
            let right = build_subline(l.words[split_idx..].to_vec(), l.baseline_y, l.avg_font_size_pt);
            let right_is_short_amount =
                is_numeric_or_currency_cell(&right.plain_text()) && right.plain_text().len() <= 16;
            if !right_is_short_amount {
                split_lines.push(left);
                split_lines.push(right);
                continue;
            }
        }
        split_lines.push(l.clone());
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum LineCol {
        Full,
        Left,
        Right,
    }

    let classify = |l: &ReconstructedLine| -> LineCol {
        if l.avg_font_size_pt >= 16.0 {
            return LineCol::Full;
        }
        if l.bbox.x1 <= page_mid + 14.0 {
            LineCol::Left
        } else if l.bbox.x0 >= page_mid - 26.0 {
            LineCol::Right
        } else {
            LineCol::Full
        }
    };

    let crosses_h_rule = |y_a: f32, y_b: f32| -> bool {
        let min_y = y_a.min(y_b);
        let max_y = y_a.max(y_b);
        h_rule_ys.iter().any(|&ry| ry >= min_y - 1.0 && ry <= max_y + 1.0)
    };

    // A Left or Right line is only part of a 2-column / side-by-side zone if there is at least
    // one line in the opposite column within 18 pt vertically (without crossing a horizontal rule);
    // otherwise it is a normal single-column line.
    let is_two_col_line: Vec<bool> = (0..split_lines.len())
        .map(|i| {
            let ci = classify(&split_lines[i]);
            if ci == LineCol::Full {
                return false;
            }
            let yi0 = split_lines[i].bbox.y0;
            let yi1 = split_lines[i].bbox.y1;
            let yi_mid = split_lines[i].bbox.center_y();
            split_lines.iter().enumerate().any(|(j, other)| {
                if i == j {
                    return false;
                }
                let cj = classify(other);
                let opposite = (ci == LineCol::Left && cj == LineCol::Right)
                    || (ci == LineCol::Right && cj == LineCol::Left);
                opposite
                    && other.bbox.y0 <= yi1 + 18.0
                    && other.bbox.y1 >= yi0 - 18.0
                    && !crosses_h_rule(yi_mid, other.bbox.center_y())
            })
        })
        .collect();

    let mut out_blocks: Vec<(f32, FlowBlock)> = Vec::new();
    let mut layout_table_count = 0usize;
    let mut idx = 0;

    while idx < split_lines.len() {
        if !is_two_col_line[idx] {
            let start = idx;
            idx += 1;
            while idx < split_lines.len() && !is_two_col_line[idx] {
                idx += 1;
            }
            for p in group_lines_into_paragraphs(
                &split_lines[start..idx],
                params,
                margin_left_pt,
                content_width_pt,
            ) {
                out_blocks.push((p.source_bbox.y0, FlowBlock::Paragraph(p)));
            }
            continue;
        }

        // Collect contiguous 2-column lines (Left and Right column lines in this vertical band),
        // breaking if there is a vertical section gap (> 14 pt bbox gap) or a horizontal divider line.
        let start = idx;
        let mut left_lines = Vec::new();
        let mut right_lines = Vec::new();
        let mut band_max_y1 = split_lines[idx].bbox.y1;
        while idx < split_lines.len() && is_two_col_line[idx] {
            if idx > start
                && (split_lines[idx].bbox.y0 - band_max_y1 > 14.0
                    || crosses_h_rule(band_max_y1, split_lines[idx].bbox.y0))
            {
                break;
            }
            band_max_y1 = band_max_y1.max(split_lines[idx].bbox.y1);
            match classify(&split_lines[idx]) {
                LineCol::Left => left_lines.push(split_lines[idx].clone()),
                LineCol::Right => right_lines.push(split_lines[idx].clone()),
                LineCol::Full => unreachable!(),
            }
            idx += 1;
        }

        // Absorb immediately following Left or Right column lines that continue the column
        // after the opposite column has ended (e.g. a company registration line in an invoice footer)
        if !left_lines.is_empty() && !right_lines.is_empty() {
            while idx < split_lines.len() {
                let next_col = classify(&split_lines[idx]);
                let absorb_left = next_col == LineCol::Left && {
                    let last_l = left_lines.last().unwrap();
                    let dy = split_lines[idx].bbox.y0 - last_l.bbox.y1;
                    dy >= -1.0
                        && dy <= 8.0
                        && (split_lines[idx].bbox.x0 - left_lines[0].bbox.x0).abs() <= 4.0
                        && !crosses_h_rule(last_l.bbox.center_y(), split_lines[idx].bbox.center_y())
                };
                let absorb_right = next_col == LineCol::Right && {
                    let last_r = right_lines.last().unwrap();
                    let dy = split_lines[idx].bbox.y0 - last_r.bbox.y1;
                    dy >= -1.0
                        && dy <= 8.0
                        && (split_lines[idx].bbox.x0 - right_lines[0].bbox.x0).abs() <= 4.0
                        && !crosses_h_rule(last_r.bbox.center_y(), split_lines[idx].bbox.center_y())
                };
                if absorb_left {
                    band_max_y1 = band_max_y1.max(split_lines[idx].bbox.y1);
                    left_lines.push(split_lines[idx].clone());
                    idx += 1;
                } else if absorb_right {
                    band_max_y1 = band_max_y1.max(split_lines[idx].bbox.y1);
                    right_lines.push(split_lines[idx].clone());
                    idx += 1;
                } else {
                    break;
                }
            }
        }

        if left_lines.is_empty() || right_lines.is_empty() {
            // Only one side is populated (e.g. short left-aligned lines or right-aligned date/signature block)
            let combined = &split_lines[start..idx];
            for p in group_lines_into_paragraphs(combined, params, margin_left_pt, content_width_pt) {
                out_blocks.push((p.source_bbox.y0, FlowBlock::Paragraph(p)));
            }
            continue;
        }

        let band_y0 = left_lines[0].bbox.y0.min(right_lines[0].bbox.y0);
        let band_y1 = left_lines
            .last()
            .unwrap()
            .bbox
            .y1
            .max(right_lines.last().unwrap().bbox.y1);

        // Distinguish a Side-by-Side Header/Address/Totals Block (Sender on left + Recipient on right,
        // or Payment Terms on left + Totals on right) from a 2-Column Flowing Article/Contract Body!
        let is_top_header_address_zone = (band_y0 <= page_height_pt * 0.36
            && left_lines.len() <= 12
            && right_lines.len() <= 12)
            || (left_lines.len() <= 6 && right_lines.len() <= 6);

        let left_min_x0 = left_lines
            .iter()
            .map(|l| l.bbox.x0)
            .fold(f32::INFINITY, f32::min);
        let left_col_indent = if left_min_x0.is_finite() && left_min_x0 >= margin_left_pt + 3.0 {
            (left_min_x0 - margin_left_pt).clamp(0.0, 50.0)
        } else {
            0.0
        };
        let right_margin_x = right_lines
            .iter()
            .map(|l| l.bbox.x0)
            .fold(f32::INFINITY, f32::min);
        let right_margin_x = if right_margin_x.is_finite() {
            right_margin_x
        } else {
            page_mid
        };
        let eff_width = (content_width_pt - left_col_indent).max(160.0);
        let left_col_w = (right_margin_x - (margin_left_pt + left_col_indent))
            .clamp(eff_width * 0.35, eff_width * 0.72);
        let right_col_w = (eff_width - left_col_w).max(110.0);

        let (left_shd, left_bdr) = detect_half_box_style(page, &left_lines);
        let (right_shd, right_bdr) = detect_half_box_style(page, &right_lines);

        if is_top_header_address_zone {
            // In a top Sender/Recipient or bottom Payment/Totals block, keep each line as its own paragraph
            let left_paras: Vec<Paragraph> = left_lines
                .iter()
                .map(|l| line_to_col_paragraph(l, params, left_col_w))
                .collect();
            let right_paras: Vec<Paragraph> = right_lines
                .iter()
                .map(|l| line_to_col_paragraph(l, params, right_col_w))
                .collect();
            let cell_left = TableCell {
                paragraphs: if left_paras.is_empty() {
                    vec![Paragraph::default()]
                } else {
                    left_paras
                },
                grid_span: 1,
                v_merge: None,
                width_pt: left_col_w,
                shading_color: left_shd,
                border_top: left_bdr,
                border_bottom: left_bdr,
                border_left: left_bdr,
                border_right: left_bdr,
            };
            let cell_right = TableCell {
                paragraphs: if right_paras.is_empty() {
                    vec![Paragraph::default()]
                } else {
                    right_paras
                },
                grid_span: 1,
                v_merge: None,
                width_pt: right_col_w,
                shading_color: right_shd,
                border_top: right_bdr,
                border_bottom: right_bdr,
                border_left: right_bdr,
                border_right: right_bdr,
            };
            let layout_tbl = TableGrid {
                col_widths_pt: vec![left_col_w, right_col_w],
                rows: vec![TableRow {
                    cells: vec![cell_left, cell_right],
                    min_height_pt: 20.0,
                    is_header_row: false,
                }],
                left_indent_pt: left_col_indent,
                space_before_pt: 0.0,
                space_after_pt: 0.0,
                is_borderless_layout: true,
                source_bbox: RectPt::new(
                    margin_left_pt + left_col_indent,
                    band_y0,
                    margin_left_pt + left_col_indent + left_col_w + right_col_w,
                    band_y1,
                ),
            };
            layout_table_count += 1;
            out_blocks.push((band_y0, FlowBlock::Table(layout_tbl)));
        } else {
            // 2-Column Flowing Body: emit as a 2-column borderless TableGrid so Word (.docx) and HTML
            // display the two columns side-by-side exactly like the PDF, while `write_txt` still reads
            // all of `cell_left` (Column 1) before `cell_right` (Column 2)!
            let mut left_paras =
                group_lines_into_paragraphs(&left_lines, params, margin_left_pt + left_col_indent, left_col_w);
            let mut right_paras =
                group_lines_into_paragraphs(&right_lines, params, right_margin_x, right_col_w);
            for p in left_paras.iter_mut().chain(right_paras.iter_mut()) {
                p.alignment = Alignment::Left;
                p.left_indent_pt = 0.0;
                p.space_after_pt = 0.0;
            }
            let cell_left = TableCell {
                paragraphs: if left_paras.is_empty() {
                    vec![Paragraph::default()]
                } else {
                    left_paras
                },
                grid_span: 1,
                v_merge: None,
                width_pt: left_col_w,
                shading_color: left_shd,
                border_top: left_bdr,
                border_bottom: left_bdr,
                border_left: left_bdr,
                border_right: left_bdr,
            };
            let cell_right = TableCell {
                paragraphs: if right_paras.is_empty() {
                    vec![Paragraph::default()]
                } else {
                    right_paras
                },
                grid_span: 1,
                v_merge: None,
                width_pt: right_col_w,
                shading_color: right_shd,
                border_top: right_bdr,
                border_bottom: right_bdr,
                border_left: right_bdr,
                border_right: right_bdr,
            };
            let two_col_tbl = TableGrid {
                col_widths_pt: vec![left_col_w, right_col_w],
                rows: vec![TableRow {
                    cells: vec![cell_left, cell_right],
                    min_height_pt: 20.0,
                    is_header_row: false,
                }],
                left_indent_pt: left_col_indent,
                space_before_pt: 0.0,
                space_after_pt: 0.0,
                is_borderless_layout: true,
                source_bbox: RectPt::new(
                    margin_left_pt + left_col_indent,
                    band_y0,
                    margin_left_pt + left_col_indent + left_col_w + right_col_w,
                    band_y1,
                ),
            };
            out_blocks.push((band_y0, FlowBlock::Table(two_col_tbl)));
        }
    }

    (out_blocks, layout_table_count)
}

fn cluster_1d_coords(coords: &mut [f32], tol: f32) -> Vec<f32> {
    if coords.is_empty() {
        return Vec::new();
    }
    coords.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut clusters: Vec<(f32, usize)> = Vec::new();
    for &v in coords.iter() {
        if let Some((mean, count)) = clusters.last_mut() {
            if (v - *mean).abs() <= tol {
                *mean = (*mean * (*count as f32) + v) / ((*count + 1) as f32);
                *count += 1;
                continue;
            }
        }
        clusters.push((v, 1));
    }
    clusters.into_iter().map(|(m, _)| m).collect()
}

fn estimate_page_margins(
    page: &PageIr,
    lines: &[ReconstructedLine],
    tables: &[TableGrid],
) -> (f32, f32, f32, f32) {
    let mut min_x = page.width_pt;
    let mut max_x = 0.0f32;
    let mut min_y = page.height_pt;
    let mut max_y = 0.0f32;

    for l in lines {
        min_x = min_x.min(l.bbox.x0);
        max_x = max_x.max(l.bbox.x1);
        min_y = min_y.min(l.bbox.y0);
        max_y = max_y.max(l.bbox.y1);
    }
    for t in tables {
        min_x = min_x.min(t.source_bbox.x0);
        max_x = max_x.max(t.source_bbox.x1);
        min_y = min_y.min(t.source_bbox.y0);
        max_y = max_y.max(t.source_bbox.y1);
    }
    for img in &page.images {
        if !img.is_full_page_scan {
            min_x = min_x.min(img.bbox.x0);
            max_x = max_x.max(img.bbox.x1);
            min_y = min_y.min(img.bbox.y0);
            max_y = max_y.max(img.bbox.y1);
        }
    }

    if min_x >= max_x || min_y >= max_y {
        return (56.7, 56.7, 56.7, 56.7);
    }

    let left = min_x.clamp(12.0, 90.0);
    let right = (page.width_pt - max_x).clamp(12.0, 90.0);
    let top = min_y.clamp(12.0, 120.0);
    let bottom = (page.height_pt - max_y).clamp(12.0, 90.0);
    (left, right, top, bottom)
}

fn group_lines_into_paragraphs(
    lines: &[ReconstructedLine],
    params: &Params,
    margin_left_pt: f32,
    content_width_pt: f32,
) -> Vec<Paragraph> {
    if lines.is_empty() {
        return Vec::new();
    }

    // Estimate median body font size
    let mut sizes: Vec<f32> = lines.iter().map(|l| l.avg_font_size_pt).collect();
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let body_font_size = sizes[sizes.len() / 2].max(9.0);

    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let first_line = &lines[i];

        // Check if this line is a label + right-aligned amount or tabbed field pair
        if let Some(tab_para) = try_build_tabbed_pair_paragraph(
            first_line,
            params,
            margin_left_pt,
            content_width_pt,
        ) {
            paragraphs.push(tab_para);
            i += 1;
            continue;
        }

        let mut para_lines = vec![first_line];
        let mut j = i + 1;

        while j < lines.len() {
            let prev = para_lines.last().unwrap();
            let curr = &lines[j];

            // Don't merge across large font size changes (headings)
            if (curr.avg_font_size_pt - prev.avg_font_size_pt).abs() > 1.5 {
                break;
            }
            // Don't merge across bold/non-bold subheading transitions UNLESS the bold line is an inline
            // highlighted sentence continuation inside a wrapped prose paragraph (e.g. page 2 of a slide-style course)!
            let prev_bold = prev.words.iter().all(|w| w.style.bold);
            let curr_bold = curr.words.iter().all(|w| w.style.bold);
            if prev_bold != curr_bold {
                let prev_txt = prev.plain_text();
                let prev_trim = prev_txt.trim_end();
                let prev_continues_sentence = !prev_trim.is_empty()
                    && !prev_trim.ends_with('.')
                    && !prev_trim.ends_with('!')
                    && !prev_trim.ends_with('?')
                    && !prev_trim.ends_with(':')
                    && prev.bbox.width() >= content_width_pt * 0.65;
                if !prev_continues_sentence {
                    break;
                }
            }
            // Don't merge if current line starts a bullet (text or vector) or numbered list item
            if curr.has_vector_bullet || detect_list_prefix(&curr.plain_text()).is_some() {
                break;
            }
            // Don't merge if current line is a tabbed label-amount pair
            if try_build_tabbed_pair_paragraph(curr, params, margin_left_pt, content_width_pt)
                .is_some()
            {
                break;
            }

            let baseline_step = curr.baseline_y - prev.baseline_y;
            let max_para_step = prev.avg_font_size_pt * 1.78;
            let left_diff = (curr.bbox.x0 - prev.bbox.x0).abs();
            // Only merge into a wrapped paragraph if the previous line extended across at least
            // 65% of the available width from `prev.bbox.x0` to the right margin; short lines
            // (addresses, salutations, `Key : Value` invoice conditions) stay separate!
            let avail_w = (margin_left_pt + content_width_pt - prev.bbox.x0.max(margin_left_pt))
                .max(120.0);
            let min_wrap_w = (avail_w * 0.65).min(290.0).max(115.0);
            let prev_wrapped = prev.bbox.width() >= min_wrap_w;

            // Merge if vertical spacing is normal single/1.15 spacing, left margin aligns, and prev wrapped
            if baseline_step > 0.0
                && baseline_step <= max_para_step
                && left_diff <= 24.0
                && prev_wrapped
            {
                para_lines.push(curr);
                j += 1;
            } else {
                break;
            }
        }

        let mut para = merge_lines_to_paragraph(
            &para_lines,
            params,
            body_font_size,
            margin_left_pt,
            content_width_pt,
        );

        // Compute space_before from vertical gap above this paragraph
        if let Some(prev_para) = paragraphs.last() {
            let v_gap = (para.source_bbox.y0 - prev_para.source_bbox.y1 - 1.5).max(0.0);
            para.space_before_pt = v_gap.clamp(0.0, 28.0);
        }

        paragraphs.push(para);
        i = j;
    }

    paragraphs
}

fn try_build_tabbed_pair_paragraph(
    line: &ReconstructedLine,
    params: &Params,
    margin_left_pt: f32,
    content_width_pt: f32,
) -> Option<Paragraph> {
    if line.words.len() < 2 {
        return None;
    }
    // Find largest horizontal gap between adjacent words on this line
    let mut best_split: Option<(usize, f32)> = None;
    for idx in 0..(line.words.len() - 1) {
        let gap = line.words[idx + 1].bbox.x0 - line.words[idx].bbox.x1;
        let next_is_colon = line.words[idx + 1].text.starts_with(':');
        if gap >= 38.0 || (next_is_colon && gap >= 18.0) {
            if best_split.is_none_or(|(_, best_g)| gap > best_g) {
                best_split = Some((idx, gap));
            }
        }
    }

    let (split_idx, gap) = best_split?;
    let left_words = &line.words[..=split_idx];
    let right_words = &line.words[split_idx + 1..];

    let mut runs = words_to_runs(left_words);
    let tab_style = runs.last().map(|r| r.style.clone()).unwrap_or_default();
    runs.push(TextRun {
        text: "\t".to_string(),
        style: tab_style,
    });
    runs.extend(words_to_runs(right_words));

    let right_txt: String = right_words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let reaches_right_margin = line.bbox.x1 >= margin_left_pt + content_width_pt * 0.76;
    let is_right_aligned_pair = reaches_right_margin
        && (is_numeric_or_currency_cell(&right_txt)
            || is_page_number_footer(&right_txt)
            || gap >= 110.0);

    let left_indent = (line.bbox.x0 - margin_left_pt).max(0.0);
    let tab_stop = if is_right_aligned_pair {
        TabStop {
            pos_pt: (line.bbox.x1 - margin_left_pt).clamp(content_width_pt * 0.5, content_width_pt),
            align: TabAlignment::Right,
            leader: TabLeader::None,
        }
    } else {
        TabStop {
            pos_pt: (right_words[0].bbox.x0 - margin_left_pt).max(left_indent + 36.0),
            align: TabAlignment::Left,
            leader: TabLeader::None,
        }
    };

    let line_spacing = compute_line_spacing(line.avg_font_size_pt, line.avg_font_size_pt * 1.2, params);

    Some(Paragraph {
        runs,
        alignment: Alignment::Left,
        space_before_pt: 0.0,
        space_after_pt: 0.0,
        left_indent_pt: left_indent,
        right_indent_pt: 0.0,
        first_line_indent_pt: 0.0,
        line_spacing,
        heading_level: None,
        list_kind: None,
        tab_stops: vec![tab_stop],
        page_break_before: false,
        keep_with_next: false,
        auto_hyphenation: false,
        source_bbox: line.bbox,
    })
}

fn line_to_paragraph(line: &ReconstructedLine, params: &Params, page_width_pt: f32) -> Paragraph {
    let runs = words_to_runs(&line.words);
    let center_offset = (line.bbox.center_x() - 0.5 * page_width_pt).abs();
    let alignment = if center_offset <= 28.0 && line.bbox.width() < page_width_pt * 0.75 {
        Alignment::Center
    } else if line.bbox.x0 > page_width_pt * 0.55 {
        Alignment::Right
    } else {
        Alignment::Left
    };

    Paragraph {
        runs,
        alignment,
        space_before_pt: 0.0,
        space_after_pt: 0.0,
        left_indent_pt: 0.0,
        right_indent_pt: 0.0,
        first_line_indent_pt: 0.0,
        line_spacing: compute_line_spacing(line.avg_font_size_pt, line.avg_font_size_pt * 1.18, params),
        heading_level: None,
        list_kind: None,
        tab_stops: Vec::new(),
        page_break_before: false,
        keep_with_next: false,
        auto_hyphenation: false,
        source_bbox: line.bbox,
    }
}

fn merge_lines_to_paragraph(
    para_lines: &[&ReconstructedLine],
    params: &Params,
    body_font_size: f32,
    margin_left_pt: f32,
    content_width_pt: f32,
) -> Paragraph {
    let mut bbox = para_lines[0].bbox;
    for l in &para_lines[1..] {
        bbox = bbox.union(&l.bbox);
    }

    let avg_font_size = para_lines.iter().map(|l| l.avg_font_size_pt).sum::<f32>()
        / (para_lines.len() as f32);

    let measured_leading = if para_lines.len() >= 2 {
        let total_step = para_lines.last().unwrap().baseline_y - para_lines[0].baseline_y;
        (total_step / ((para_lines.len() - 1) as f32)).max(avg_font_size * 1.05)
    } else {
        avg_font_size * 1.18
    };

    let line_spacing = compute_line_spacing(avg_font_size, measured_leading, params);

    // Join words across lines, dehyphenating French end-of-line hyphens when followed by a lowercase word
    let mut all_words: Vec<WordCluster> = Vec::new();
    let mut saw_hyphen = false;

    for (line_idx, l) in para_lines.iter().enumerate() {
        for (w_idx, w) in l.words.iter().enumerate() {
            if w_idx == 0 && line_idx > 0 {
                if let Some(prev_w) = all_words.last_mut() {
                    if prev_w.text.ends_with('-')
                        && prev_w.text.len() >= 3
                        && w.text.chars().next().is_some_and(|c| c.is_lowercase())
                    {
                        prev_w.text.pop(); // remove soft line-break hyphen
                        prev_w.text.push_str(&w.text);
                        prev_w.bbox = prev_w.bbox.union(&w.bbox);
                        saw_hyphen = true;
                        continue;
                    }
                }
            }
            all_words.push(w.clone());
        }
    }

    let heading_level = if avg_font_size >= body_font_size * 1.55 {
        Some(1)
    } else if avg_font_size >= body_font_size * 1.25 {
        Some(2)
    } else {
        None
    };

    let first_text = para_lines[0].plain_text();
    let text_list_kind = if heading_level.is_none() {
        detect_list_prefix(&first_text)
    } else {
        None
    };
    let list_kind = text_list_kind.or_else(|| {
        if para_lines[0].has_vector_bullet {
            Some(ListKind::Bullet)
        } else {
            None
        }
    });
    if text_list_kind.is_some() && !all_words.is_empty() {
        // Strip the bullet or number token from the first word if it's a standalone marker
        let first_token = all_words[0].text.trim();
        if matches!(first_token, "•" | "-" | "–" | "▪")
            || (first_token.len() <= 4
                && (first_token.ends_with('.') || first_token.ends_with(')'))
                && first_token[..first_token.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric()))
        {
            all_words.remove(0);
        }
    }

    let runs = words_to_runs(&all_words);

    // Alignment heuristic
    let page_center = margin_left_pt + 0.5 * content_width_pt;
    let center_diff = (bbox.center_x() - page_center).abs();
    let is_justified = list_kind.is_none()
        && para_lines.len() >= 2
        && para_lines[..para_lines.len() - 1]
            .iter()
            .all(|l| l.bbox.width() >= content_width_pt * 0.85);
    let is_colon_kv = first_text.contains(" : ") || first_text.contains(": ");

    let alignment = if is_justified {
        Alignment::Justify
    } else if para_lines.len() == 1
        && !is_colon_kv
        && center_diff <= 16.0
        && bbox.width() < content_width_pt * 0.78
        && bbox.x0 < page_center - 16.0
        && bbox.x1 > page_center + 16.0
    {
        Alignment::Center
    } else if para_lines.len() == 1
        && !is_colon_kv
        && bbox.x0 > margin_left_pt + content_width_pt * 0.70
        && (margin_left_pt + content_width_pt - bbox.x1).abs() <= 32.0
    {
        Alignment::Right
    } else {
        Alignment::Left
    };

    let left_indent_pt = if alignment == Alignment::Left {
        let raw_indent = (bbox.x0 - margin_left_pt).max(0.0);
        if raw_indent >= 3.0 && raw_indent <= content_width_pt * 0.95 {
            raw_indent
        } else {
            0.0
        }
    } else {
        0.0
    };

    Paragraph {
        runs,
        alignment,
        space_before_pt: 0.0,
        space_after_pt: 0.0,
        left_indent_pt,
        right_indent_pt: 0.0,
        first_line_indent_pt: 0.0,
        line_spacing,
        heading_level,
        list_kind,
        tab_stops: Vec::new(),
        page_break_before: false,
        keep_with_next: heading_level.is_some(),
        auto_hyphenation: saw_hyphen && is_justified,
        source_bbox: bbox,
    }
}

fn compute_line_spacing(font_size_pt: f32, measured_leading_pt: f32, params: &Params) -> LineSpacingRule {
    match params.preset {
        Preset::Editable => {
            // Critic 1 fix: never emit `Exact` line spacing in Editable mode!
            // Use `AtLeast` with a minimum factor of `1.15 * font_size_pt` so enlarging text by +4 pt in Pages never clips!
            let min_pt = (font_size_pt * params.min_editable_line_factor).max(measured_leading_pt);
            LineSpacingRule::AtLeastPt((min_pt * 2.0).round() / 2.0)
        }
        Preset::Fidele | Preset::Exact => LineSpacingRule::ExactPt((measured_leading_pt * 2.0).round() / 2.0),
    }
}

fn detect_list_prefix(text: &str) -> Option<ListKind> {
    let trimmed = text.trim_start();
    if trimmed.starts_with("• ")
        || trimmed.starts_with("- ")
        || trimmed.starts_with("– ")
        || trimmed.starts_with("▪ ")
    {
        return Some(ListKind::Bullet);
    }
    if let Some((first_tok, _)) = trimmed.split_once(' ') {
        if first_tok.len() >= 2
            && first_tok.len() <= 4
            && (first_tok.ends_with('.') || first_tok.ends_with(')'))
            && first_tok[..first_tok.len() - 1]
                .chars()
                .all(|c| c.is_ascii_digit())
        {
            return Some(ListKind::OrderedDecimal);
        }
    }
    None
}

fn words_to_runs(words: &[WordCluster]) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let prefix = if i > 0 { " " } else { "" };
        if let Some(last) = runs.last_mut() {
            if last.style == w.style {
                last.text.push_str(prefix);
                last.text.push_str(&w.text);
                continue;
            }
        }
        runs.push(TextRun {
            text: format!("{prefix}{}", w.text),
            style: w.style.clone(),
        });
    }
    runs
}

/// Merge continuation tables across consecutive pages when column counts and widths match.
fn merge_cross_page_tables(sections: &mut Vec<Section>) {
    let mut idx = 0;
    while idx + 1 < sections.len() {
        let can_merge = {
            let sec_a = &sections[idx];
            let sec_b = &sections[idx + 1];
            match (sec_a.blocks.last(), sec_b.blocks.first()) {
                (Some(FlowBlock::Table(ta)), Some(FlowBlock::Table(tb))) => {
                    ta.col_widths_pt.len() == tb.col_widths_pt.len()
                        && ta
                            .col_widths_pt
                            .iter()
                            .zip(tb.col_widths_pt.iter())
                            .all(|(wa, wb)| (wa - wb).abs() <= 12.0)
                }
                _ => false,
            }
        };

        if can_merge {
            if let Some(FlowBlock::Table(tb)) = sections[idx + 1].blocks.first().cloned() {
                sections[idx + 1].blocks.remove(0);
                if let Some(FlowBlock::Table(ta)) = sections[idx].blocks.last_mut() {
                    // Skip repeated header row on continuation page if identical to first row
                    let start_row = if !ta.rows.is_empty()
                        && !tb.rows.is_empty()
                        && row_text_signature(&ta.rows[0]) == row_text_signature(&tb.rows[0])
                    {
                        1
                    } else {
                        0
                    };
                    ta.rows.extend_from_slice(&tb.rows[start_row..]);
                }
                if sections[idx + 1].blocks.is_empty() {
                    sections.remove(idx + 1);
                    continue;
                }
            }
        }
        idx += 1;
    }
}

fn row_text_signature(row: &TableRow) -> String {
    row.cells
        .iter()
        .map(|c| {
            c.paragraphs
                .iter()
                .flat_map(|p| p.runs.iter().map(|r| r.text.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .collect::<Vec<_>>()
        .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracked_heading_not_split() {
        let letters = ["F", "A", "C", "T", "U", "R", "E"];
        let mut glyphs = Vec::new();
        let mut x = 60.0f32;
        for &ch in &letters {
            glyphs.push(GlyphIr {
                unicode: ch.to_string(),
                glyph_id: 1,
                bbox: RectPt::new(x, 70.0, x + 9.0, 86.0),
                baseline_y: 84.0,
                font_size_pt: 16.0,
                advance_pt: 9.0,
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
            x += 9.0 + 3.5;
        }
        let doc = DocIr {
            pages: vec![PageIr {
                page_index: 0,
                width_pt: 595.28,
                height_pt: 841.89,
                rotation_deg: 0,
                fonts: vec![FontIr::default()],
                glyphs,
                paths: Vec::new(),
                images: Vec::new(),
                annotations: Vec::new(),
                form_fields: Vec::new(),
                page_raster_png: None,
                graphics_png: None,
                graphics_format: String::new(),
                has_complex_graphics: false,
            }],
            has_acroform: false,
            producer: None,
            title: None,
        };
        let (flow, report) = reconstruct_document(&doc, &Params::default());
        assert_eq!(report.tracked_heading_fixes, 1);
        let FlowBlock::Paragraph(p) = &flow.sections[0].blocks[0] else {
            panic!("Expected paragraph");
        };
        assert_eq!(p.runs[0].text, "FACTURE");
    }

    #[test]
    fn test_skia_zwsp_and_nnbsp_normalization() {
        let mut fixes = 0;
        let out = normalize_unicode_str(
            "Bonjour\u{200B}Madame\u{202F}!",
            Profile::Pages,
            true,
            &mut fixes,
        );
        assert_eq!(out, "Bonjour Madame\u{00A0}!");
        assert_eq!(fixes, 1);
    }
}
