//! `Preset::Exact`: pixel-identical pages. Each page keeps its graphics as a background image
//! (rendered without its horizontal text) and every text block is re-created as live, editable
//! text in a box anchored at its exact PDF position.
//!
//! Placement follows how Pages lays out DOCX text boxes, measured by rendering calibration
//! documents back to PDF: with `Exact` line spacing `L` the first baseline sits `0.8 × L` below
//! the top of the box and each following line `L` lower, and text starts 0.5 pt right of the
//! box edge. Word positions inside a line are matched with character spacing on each run.

use p2d_ir::*;

use crate::fonts::{FontMetrics, resolve_exact_font};
use crate::{compute_2means_word_gap_threshold, normalize_unicode_str};

/// Share of an `Exact` line height that sits above the baseline (Pages and Word).
pub const EXACT_BASELINE_RATIO: f32 = 0.8;
/// Pages starts text-box content this far from each side of the box, even with zero insets.
pub const TEXTBOX_X_INSET_PT: f32 = 0.5;
/// Glyphs rotated by more than this stay in the page background (see `p2d_pdf::TEXT_SKIP_MAX_ANGLE_DEG`).
pub const EXACT_TEXT_MAX_ANGLE_DEG: f32 = 2.0;

/// A run of glyphs on one baseline with no column-sized gap inside.
struct Segment<'a> {
    glyphs: Vec<&'a GlyphIr>,
    baseline: f32,
    size: f32,
    x0: f32,
    x1: f32,
}

/// Consecutive segments forming one paragraph-like block (same size, left edge and pitch).
struct Block<'a> {
    lines: Vec<Segment<'a>>,
    align: Alignment,
}

impl Block<'_> {
    fn pitch(&self) -> Option<f32> {
        if self.lines.len() < 2 {
            return None;
        }
        let mut deltas: Vec<f32> = self.lines.windows(2).map(|w| w[1].baseline - w[0].baseline).collect();
        deltas.sort_by(f32::total_cmp);
        Some(deltas[deltas.len() / 2])
    }

    fn x0(&self) -> f32 {
        self.lines.iter().map(|l| l.x0).fold(f32::MAX, f32::min)
    }

    fn x1(&self) -> f32 {
        self.lines.iter().map(|l| l.x1).fold(f32::MIN, f32::max)
    }
}

/// Build the `Exact` document: one section per page with its background and text boxes.
pub fn reconstruct_exact(doc: &DocIr, params: &Params, metrics: &dyn FontMetrics) -> (FlowDoc, ConvertReport) {
    let mut report = ConvertReport {
        page_count: doc.pages.len(),
        has_acroform: doc.has_acroform,
        form_field_count: doc.pages.iter().map(|p| p.form_fields.len()).sum(),
        ..Default::default()
    };
    let mut sections = Vec::with_capacity(doc.pages.len());
    let mut media_id = 1usize;

    for (page_index, page) in doc.pages.iter().enumerate() {
        p2d_ir::progress::report(p2d_ir::progress::Step::Layout { page: page_index + 1, pages: doc.pages.len() });
        let has_visible_text = page.glyphs.iter().any(is_visible_glyph);
        let glyphs: Vec<&GlyphIr> = page
            .glyphs
            .iter()
            .filter(|g| has_visible_text && is_visible_glyph(g) && g.angle_deg.abs() <= EXACT_TEXT_MAX_ANGLE_DEG)
            .collect();

        let segments: Vec<Segment<'_>> = group_lines(glyphs).into_iter().flat_map(split_segments).collect();
        let blocks = group_blocks(segments);

        let mut out_blocks = Vec::with_capacity(blocks.len() + 1);
        if let Some(bg) = &page.graphics_png {
            out_blocks.push(FlowBlock::Image(AnchoredImage {
                media_id,
                format: if page.graphics_format.is_empty() {
                    "png".to_string()
                } else {
                    page.graphics_format.clone()
                },
                data: bg.clone(),
                bbox_pt: RectPt::new(0.0, 0.0, page.width_pt, page.height_pt),
                behind_doc: true,
                inline: false,
            }));
            media_id += 1;
        } else if !page.has_complex_graphics {
            out_blocks.extend(page_shapes(page).into_iter().map(FlowBlock::Shape));
        }
        let page_fonts: Vec<TargetFont> = (0..page.fonts.len())
            .map(|i| target_font(page, i, metrics, params.profile))
            .collect();
        let mut word_count = 0usize;
        for block in &blocks {
            let tb = block_to_textbox(block, page, &page_fonts, params, metrics, &mut report.glued_word_fixes);
            word_count += tb
                .paragraphs
                .iter()
                .flat_map(|p| &p.runs)
                .map(|r| r.text.split_whitespace().count())
                .sum::<usize>();
            out_blocks.push(FlowBlock::TextBox(tb));
        }
        report.total_words += word_count;

        sections.push(Section {
            page_width_pt: page.width_pt,
            page_height_pt: page.height_pt,
            margin_top_pt: 0.0,
            margin_bottom_pt: 0.0,
            margin_left_pt: 0.0,
            margin_right_pt: 0.0,
            blocks: out_blocks,
            ..Default::default()
        });
        report.pages.push(PageReport {
            page_index: page.page_index,
            detected_case: crate::classify_page(page),
            rung_used: FallbackRung::FullPipeline,
            glyph_count: page.glyphs.len(),
            word_count,
            table_count: 0,
            unmapped_glyph_ratio: 0.0,
            warnings: Vec::new(),
        });
    }

    let flow_doc = FlowDoc {
        title: doc.title.clone(),
        default_font_family: "Helvetica".to_string(),
        default_font_size_pt: 11.0,
        sections,
        source_pages: Vec::new(),
    };
    (flow_doc, report)
}

/// The page's rectangles and rules as vector shapes, in drawing order. Stroked rules become bars
/// as thick as the stroke; a white backdrop covering the page is dropped.
fn page_shapes(page: &PageIr) -> Vec<ShapeBlock> {
    let page_area = page.width_pt * page.height_pt;
    page.paths
        .iter()
        .filter_map(|path| {
            let b = path.bbox;
            let thin_h = b.height() < 0.1;
            let thin_w = b.width() < 0.1;
            if thin_h && thin_w {
                return None;
            }
            if path.fill_color.is_none() && (thin_h || thin_w) {
                let color = path.stroke_color?;
                let half = 0.5 * path.stroke_width_pt.max(0.1);
                let bbox = if thin_h {
                    RectPt::new(b.x0, b.y0 - half, b.x1, b.y1 + half)
                } else {
                    RectPt::new(b.x0 - half, b.y0, b.x1 + half, b.y1)
                };
                return Some(ShapeBlock {
                    bbox_pt: bbox,
                    fill_color: Some(color),
                    stroke: None,
                });
            }
            let backdrop = path.stroke_color.is_none()
                && path.fill_color.is_some_and(|c| c.is_near_white())
                && b.area() >= 0.9 * page_area;
            if backdrop {
                return None;
            }
            Some(ShapeBlock {
                bbox_pt: b,
                fill_color: path.fill_color,
                stroke: path.stroke_color.map(|color| CellBorder {
                    width_pt: path.stroke_width_pt,
                    color,
                }),
            })
        })
        .collect()
}

/// Size set in the PDF for glyphs of `font` drawn at `visual_size` (see `FontIr::size_scale`).
fn nominal_size(font: Option<&FontIr>, visual_size: f32) -> f32 {
    match font.and_then(|f| f.size_scale).filter(|s| *s > 0.0) {
        Some(scale) => visual_size / scale,
        None => visual_size,
    }
}

fn is_visible_glyph(g: &GlyphIr) -> bool {
    g.is_visible_in_clip && g.render_mode != RenderMode::Invisible && g.color.a > 0 && !g.unicode.is_empty()
}

/// Cluster glyphs sharing a baseline (and a compatible size) into lines sorted left to right.
fn group_lines(mut glyphs: Vec<&GlyphIr>) -> Vec<Vec<&GlyphIr>> {
    glyphs.sort_by(|a, b| {
        a.baseline_y
            .total_cmp(&b.baseline_y)
            .then(a.bbox.x0.total_cmp(&b.bbox.x0))
    });
    let mut lines: Vec<(f32, f32, Vec<&GlyphIr>)> = Vec::new();
    for g in glyphs {
        let size = g.font_size_pt.max(1.0);
        let found = lines.iter_mut().rev().take(6).find(|(base, sz, _)| {
            let tol = 0.2 * sz.min(size);
            (g.baseline_y - *base).abs() <= tol && sz.max(size) <= sz.min(size) * 1.6
        });
        match found {
            Some((_, _, line)) => line.push(g),
            None => lines.push((g.baseline_y, size, vec![g])),
        }
    }
    lines
        .into_iter()
        .map(|(_, _, mut line)| {
            line.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
            // Drop glyphs printed twice at (almost) the same spot, e.g. shadowed or fake-bold text.
            let mut kept: Vec<&GlyphIr> = Vec::with_capacity(line.len());
            for g in line {
                let dup =
                    kept.iter().rev().take(3).any(|k| {
                        k.unicode == g.unicode && (k.bbox.x0 - g.bbox.x0).abs() < 0.25 * g.advance_pt.max(1.0)
                    });
                if !dup {
                    kept.push(g);
                }
            }
            kept
        })
        .collect()
}

fn glyph_end(g: &GlyphIr) -> f32 {
    g.bbox.x0 + g.advance_pt
}

/// Split a line at column-sized gaps (tabulated values, side-by-side blocks).
fn split_segments(line: Vec<&GlyphIr>) -> Vec<Segment<'_>> {
    let mut segs: Vec<Vec<&GlyphIr>> = Vec::new();
    for g in line {
        let start_new = match segs.last().and_then(|s| s.last()) {
            None => true,
            Some(prev) => {
                let size = prev.font_size_pt.max(g.font_size_pt).max(1.0);
                let gap = g.bbox.x0 - glyph_end(prev);
                gap > 1.25 * size || g.bbox.x0 < prev.bbox.x0 - 0.5 * size
            }
        };
        if start_new {
            segs.push(vec![g]);
        } else if let Some(s) = segs.last_mut() {
            s.push(g);
        }
    }
    segs.into_iter().filter_map(segment_of).collect()
}

/// A segment of `glyphs` (left to right), whitespace at both ends trimmed.
fn segment_of(glyphs: Vec<&GlyphIr>) -> Option<Segment<'_>> {
    let first = glyphs.iter().position(|g| !is_space_glyph(g))?;
    let last = glyphs.iter().rposition(|g| !is_space_glyph(g))?;
    let glyphs = glyphs[first..=last].to_vec();
    let mut sizes: Vec<f32> = glyphs.iter().map(|g| g.font_size_pt).collect();
    sizes.sort_by(f32::total_cmp);
    let mut bases: Vec<f32> = glyphs.iter().map(|g| g.baseline_y).collect();
    bases.sort_by(f32::total_cmp);
    Some(Segment {
        baseline: bases[bases.len() / 2],
        size: sizes[sizes.len() / 2],
        x0: glyphs[0].bbox.x0,
        x1: glyph_end(glyphs[glyphs.len() - 1]),
        glyphs,
    })
}

/// A glyph that reads as a space: whitespace, or characters the text normalization turns into
/// spaces or drops.
fn is_space_glyph(g: &GlyphIr) -> bool {
    g.unicode.chars().all(crate::is_blank_char)
}

/// Cut a segment where the PDF jumps further than a word space or lines text up with a row of
/// spaces: table cells and form labels set next to each other read as one line, but each is
/// edited on its own.
fn split_at_jumps(seg: Segment<'_>) -> Vec<Segment<'_>> {
    let gaps = |after_space: bool| -> Vec<f32> {
        seg.glyphs
            .windows(2)
            .filter(|w| is_space_glyph(w[0]) == after_space && !is_space_glyph(w[1]))
            .map(|w| w[1].bbox.x0 - glyph_end(w[0]))
            .collect()
    };
    let has_spaces = seg.glyphs.iter().any(|g| is_space_glyph(g));
    let usual = median(gaps(false)).unwrap_or(0.0);
    let usual_space = median(gaps(true)).unwrap_or(0.0);
    let size = seg.size;
    let mut parts: Vec<Vec<&GlyphIr>> = Vec::new();
    let mut prev: Option<&GlyphIr> = None;
    let mut spaces_before = 0;
    for &g in &seg.glyphs {
        let jump = prev.is_some_and(|p| {
            let gap = g.bbox.x0 - glyph_end(p);
            match (is_space_glyph(p), is_space_glyph(g)) {
                (false, false) => gap > usual + if has_spaces { 0.3 * size } else { 0.8 * size },
                (true, false) => spaces_before >= 3 || gap > (usual_space + 0.5 * size).max(size),
                _ => false,
            }
        });
        match parts.last_mut() {
            Some(part) if !jump => part.push(g),
            _ => parts.push(vec![g]),
        }
        spaces_before = if is_space_glyph(g) { spaces_before + 1 } else { 0 };
        prev = Some(g);
    }
    if parts.len() == 1 {
        return vec![seg];
    }
    parts.into_iter().filter_map(segment_of).collect()
}

/// Group segments into blocks of consecutive lines that share size, pitch and an alignment edge.
fn group_blocks(mut segs: Vec<Segment<'_>>) -> Vec<Block<'_>> {
    segs.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.x0.total_cmp(&b.x0)));
    let mut blocks: Vec<Block<'_>> = Vec::new();
    for seg in segs {
        let mut best: Option<(usize, Alignment)> = None;
        for (bi, block) in blocks.iter().enumerate().rev().take(12) {
            let Some(last) = block.lines.last() else {
                continue;
            };
            let delta = seg.baseline - last.baseline;
            let size = last.size;
            if (seg.size - size).abs() > 0.08 * size || delta < 0.95 * size || delta > 1.75 * size {
                continue;
            }
            if let Some(pitch) = block.pitch()
                && (delta - pitch).abs() > (0.04 * pitch).max(0.6)
            {
                continue;
            }
            // Something else sitting between the two lines means they are not one block.
            let overlap = seg.x1.min(last.x1) - seg.x0.max(last.x0);
            if overlap <= 0.0 {
                continue;
            }
            let tol = 1.5;
            let align = if (seg.x0 - last.x0).abs() <= tol || block.lines.len() == 1 && first_line_indent(last, &seg) {
                Alignment::Left
            } else if (seg.x1 - last.x1).abs() <= tol {
                Alignment::Right
            } else if ((seg.x0 + seg.x1) - (last.x0 + last.x1)).abs() <= 2.0 * tol {
                Alignment::Center
            } else {
                continue;
            };
            if block.lines.len() >= 2 && align != block.align {
                continue;
            }
            best = Some((bi, align));
            break;
        }
        match best {
            Some((bi, align)) => {
                let block = &mut blocks[bi];
                if block.lines.len() == 1 {
                    block.align = align;
                }
                block.lines.push(seg);
            }
            None => blocks.push(Block {
                lines: vec![seg],
                align: Alignment::Left,
            }),
        }
    }
    blocks
}

/// The second line starts left of an indented first line (paragraph with first-line indent).
fn first_line_indent(first: &Segment<'_>, second: &Segment<'_>) -> bool {
    let indent = first.x0 - second.x0;
    indent >= 4.0 && indent <= 8.0 * second.size
}

/// Style shared by a sequence of glyphs.
#[derive(Clone, PartialEq)]
struct GlyphStyle {
    font_idx: usize,
    size_half_pt: i32,
    color: RgbaColor,
    /// Index of the resolved target font in the block's font list.
    slot: usize,
}

impl GlyphStyle {
    /// Font size as written to DOCX (half-point resolution).
    fn size_pt(&self) -> f32 {
        self.size_half_pt as f32 / 2.0
    }
}

fn glyph_style(g: &GlyphIr) -> GlyphStyle {
    GlyphStyle {
        font_idx: g.font_idx,
        size_half_pt: (g.font_size_pt * 2.0).round() as i32,
        color: g.color,
        slot: 0,
    }
}

/// A word (or part of one, split at style changes) with its source position.
struct Piece {
    text: String,
    /// Source x of each character of `text` (`None` for the extra characters of a ligature).
    char_x: Vec<Option<f32>>,
    x0: f32,
    x1: f32,
    style: GlyphStyle,
    /// The space separating this piece from the previous one (`None` inside a word), with the
    /// advance of the PDF's own space glyph when there was one.
    space_before: Option<(char, Option<f32>)>,
}

/// Smallest gap between two glyphs that reads as a word break. Lines that spell out their spaces
/// only get a space from a gap when it is word-sized (guessed Type3 advances otherwise read as
/// gaps inside words); lines without space glyphs split where their gaps fall into two clusters,
/// so letter-spaced headings (`F A C T U R E`) stay one word.
fn word_gap_pt(line: &Segment<'_>) -> f32 {
    if line.glyphs.iter().any(|g| is_space_glyph(g)) {
        0.45 * line.size
    } else {
        let gaps: Vec<f32> = line
            .glyphs
            .windows(2)
            .map(|w| (w[1].bbox.x0 - glyph_end(w[0])).max(0.0))
            .collect();
        compute_2means_word_gap_threshold(&gaps, line.size)
    }
}

/// Cut a line into words, splitting at spaces, visible gaps and style changes.
fn line_pieces(line: &Segment<'_>, profile: Profile, normalize_skia_zwsp: bool, fixes: &mut usize) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut prev_end: Option<f32> = None;
    let mut pending_space: Option<(char, Option<f32>)> = None;
    let gap_pt = word_gap_pt(line);
    for g in &line.glyphs {
        let text = normalize_unicode_str(&g.unicode, profile, normalize_skia_zwsp, fixes);
        let style = glyph_style(g);
        if text.trim().is_empty() {
            let ch = text.chars().next().unwrap_or(' ');
            pending_space = Some((if ch == '\u{00A0}' { ch } else { ' ' }, Some(g.advance_pt)));
            prev_end = Some(glyph_end(g));
            continue;
        }
        if pending_space.is_none() && prev_end.is_some_and(|e| g.bbox.x0 - e > gap_pt) {
            pending_space = Some((' ', None));
        }
        let char_x = text.chars().enumerate().map(|(i, _)| (i == 0).then_some(g.bbox.x0));
        let extend = pending_space.is_none() && pieces.last().is_some_and(|p| p.style == style);
        if extend {
            if let Some(p) = pieces.last_mut() {
                p.text.push_str(&text);
                p.char_x.extend(char_x);
                p.x1 = glyph_end(g);
            }
        } else {
            pieces.push(Piece {
                char_x: char_x.collect(),
                text,
                x0: g.bbox.x0,
                x1: glyph_end(g),
                style,
                space_before: if pieces.is_empty() { None } else { pending_space },
            });
        }
        pending_space = None;
        prev_end = Some(glyph_end(g));
    }
    pieces
}

/// Font chosen for one PDF font: Mac family, bold, italic, and whether it is the PDF's own family.
#[derive(Clone)]
struct TargetFont {
    family: String,
    bold: bool,
    italic: bool,
    same_family: bool,
}

fn target_font(page: &PageIr, font_idx: usize, metrics: &dyn FontMetrics, profile: Profile) -> TargetFont {
    match page.fonts.get(font_idx) {
        Some(f) => {
            let (family, same_family) = resolve_exact_font(f, metrics, profile);
            let sub_bold_handled = same_family
                && (family.ends_with(" SemiBold")
                    || family.ends_with(" ExtraBold")
                    || family.ends_with(" Black"));
            TargetFont {
                family,
                bold: f.is_bold && !sub_bold_handled,
                italic: f.is_italic,
                same_family,
            }
        }
        None => TargetFont {
            family: "Helvetica".to_string(),
            bold: false,
            italic: false,
            same_family: false,
        },
    }
}

/// One character of a line as it will be typeset, with the source position it must land on.
struct TypesetChar {
    ch: char,
    /// Source x this character must start at (`None`: wherever the previous one leads).
    src_x: Option<f32>,
    piece: usize,
    is_space: bool,
}

/// Round to the 1/20 pt resolution of DOCX character spacing.
fn round_twips(pt: f32) -> f32 {
    (pt * 20.0).round() / 20.0
}

/// Runs reproducing one line: every glyph of a font the Mac also has (same metrics) lands on its
/// source x through per-character spacing; words in a look-alike font are stretched or squeezed
/// as a whole, which keeps their letters evenly spaced; spaces absorb what is left so that each
/// word starts where it did in the PDF.
fn line_runs(pieces: &[Piece], fonts: &[&TargetFont], metrics: &dyn FontMetrics) -> Vec<TextRun> {
    let mut chars: Vec<TypesetChar> = Vec::new();
    for (pi, piece) in pieces.iter().enumerate() {
        if let Some((space, _)) = piece.space_before {
            chars.push(TypesetChar {
                ch: space,
                src_x: None,
                piece: pi - 1,
                is_space: true,
            });
        }
        for (ch, x) in piece.text.chars().zip(&piece.char_x) {
            chars.push(TypesetChar {
                ch,
                src_x: *x,
                piece: pi,
                is_space: false,
            });
        }
    }
    if chars.is_empty() {
        return Vec::new();
    }

    // Natural distance from each character to the next, measured per stretch of one font so the
    // pair kerning Pages applies is included (it never kerns across fonts).
    let mut steps = vec![0.0f32; chars.len()];
    let mut exact_metrics = vec![false; pieces.len()];
    let mut start = 0;
    while start < chars.len() {
        let piece = &pieces[chars[start].piece];
        let tf = &fonts[piece.style.slot];
        let size = piece.style.size_pt();
        let mut end = start + 1;
        while end < chars.len() {
            let other = &pieces[chars[end].piece];
            if other.style.slot != piece.style.slot || other.style.size_half_pt != piece.style.size_half_pt {
                break;
            }
            end += 1;
        }
        let text: String = chars[start..end].iter().map(|c| c.ch).collect();
        match metrics.char_steps(&tf.family, tf.bold, tf.italic, size, &text) {
            Some(measured) if measured.len() == end - start => {
                steps[start..end].copy_from_slice(&measured);
                for c in &chars[start..end] {
                    exact_metrics[c.piece] = tf.same_family;
                }
            }
            _ => {
                for (k, step) in steps.iter_mut().enumerate().take(end).skip(start) {
                    *step = fallback_step(&chars, pieces, k);
                }
            }
        }
        start = end;
    }

    // Uniform per-character spacing of the words that cannot be matched glyph by glyph.
    let uniform: Vec<f32> = pieces
        .iter()
        .enumerate()
        .map(|(pi, piece)| {
            if exact_metrics[pi] {
                return 0.0;
            }
            let idx: Vec<usize> = (0..chars.len())
                .filter(|&k| chars[k].piece == pi && !chars[k].is_space)
                .collect();
            let natural: f32 = idx.iter().map(|&k| steps[k]).sum();
            let wanted = match pieces.get(pi + 1) {
                Some(next) if next.space_before.is_none() => next.x0 - piece.x0,
                _ => piece.x1 - piece.x0,
            };
            let tracked = idx.iter().filter(|&&k| k + 1 < chars.len()).count();
            if tracked == 0 {
                0.0
            } else {
                round_twips((wanted - natural) / tracked as f32)
            }
        })
        .collect();

    let mut tracking = vec![0.0f32; chars.len()];
    let mut pos = chars[0].src_x.unwrap_or(pieces[0].x0);
    for k in 0..chars.len() - 1 {
        let next = &chars[k + 1];
        let raw = match next.src_x {
            Some(x) => round_twips(x - pos - steps[k]),
            None => uniform[chars[k].piece],
        };
        tracking[k] = raw;
        pos += steps[k] + tracking[k];
    }

    let mut runs: Vec<TextRun> = Vec::new();
    for (k, c) in chars.iter().enumerate() {
        let piece = &pieces[c.piece];
        let tf = &fonts[piece.style.slot];
        let style = RunStyle {
            font_family: tf.family.clone(),
            font_size_pt: piece.style.size_pt(),
            bold: tf.bold,
            italic: tf.italic,
            color: piece.style.color,
            char_tracking_pt: tracking[k],
            ..Default::default()
        };
        match runs.last_mut() {
            Some(last) if last.style == style => last.text.push(c.ch),
            _ => runs.push(TextRun {
                text: c.ch.to_string(),
                style,
            }),
        }
    }
    runs
}

/// Distance to the next character when the target font cannot be measured: the PDF's own.
fn fallback_step(chars: &[TypesetChar], pieces: &[Piece], k: usize) -> f32 {
    let c = &chars[k];
    let piece = &pieces[c.piece];
    if c.is_space {
        let next_piece = &pieces[c.piece + 1];
        return next_piece
            .space_before
            .and_then(|(_, adv)| adv)
            .unwrap_or(0.278 * piece.style.size_pt());
    }
    let x = c.src_x.unwrap_or(piece.x0);
    match chars.get(k + 1) {
        Some(next) if next.piece == c.piece && !next.is_space => next.src_x.map_or(0.0, |nx| nx - x),
        _ => (piece.x1 - x).max(0.0),
    }
}

fn block_to_textbox(
    block: &Block<'_>,
    page: &PageIr,
    page_fonts: &[TargetFont],
    params: &Params,
    metrics: &dyn FontMetrics,
    fixes: &mut usize,
) -> TextBoxBlock {
    let size = block.lines[0].size;
    let line_h = block.pitch().unwrap_or_else(|| (1.2 * size).max(size + 2.0));
    let first_base = block.lines[0].baseline;
    let top = first_base - EXACT_BASELINE_RATIO * line_h;

    let mut fonts: Vec<(usize, TargetFont)> = Vec::new();
    let mut any_substitute = false;
    let mut lines_runs: Vec<Vec<TextRun>> = Vec::with_capacity(block.lines.len());
    let mut widest = 0.0f32;

    for line in &block.lines {
        let mut pieces = line_pieces(line, params.profile, params.normalize_skia_zwsp, fixes);
        for piece in &mut pieces {
            let slot = match fonts.iter().position(|(i, _)| *i == piece.style.font_idx) {
                Some(slot) => slot,
                None => {
                    let tf = page_fonts
                        .get(piece.style.font_idx)
                        .cloned()
                        .unwrap_or_else(|| target_font(page, piece.style.font_idx, metrics, params.profile));
                    fonts.push((piece.style.font_idx, tf));
                    fonts.len() - 1
                }
            };
            piece.style.slot = slot;
            any_substitute |= !fonts[slot].1.same_family;
        }
        let targets: Vec<&TargetFont> = fonts.iter().map(|(_, tf)| tf).collect();
        let runs = line_runs(&pieces, &targets, metrics);
        widest = widest.max(line.x1 - block.x0());
        lines_runs.push(runs);
    }

    // One paragraph per run of lines sharing their left edge, so every line keeps its own start.
    let align = if any_substitute && block.lines.len() == 1 && block.align == Alignment::Right {
        Alignment::Left
    } else {
        block.align
    };
    let x0 = block.x0();
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut i = 0;
    while i < block.lines.len() {
        let indent = block.lines[i].x0 - x0;
        let mut j = i + 1;
        let mut first_line_indent_pt = 0.0;
        if align == Alignment::Left
            && j < block.lines.len()
            && first_line_indent(&block.lines[i], &block.lines[j])
        {
            first_line_indent_pt = block.lines[i].x0 - block.lines[j].x0;
        }
        let body_x0 = if first_line_indent_pt > 0.0 {
            block.lines[j].x0
        } else {
            block.lines[i].x0
        };
        while j < block.lines.len() && (align != Alignment::Left || (block.lines[j].x0 - body_x0).abs() <= 0.3) {
            j += 1;
        }
        let mut runs: Vec<TextRun> = Vec::new();
        for (k, line_runs) in lines_runs[i..j].iter().enumerate() {
            if k > 0
                && let Some(last) = runs.last_mut()
            {
                last.text.push('\n');
            }
            runs.extend(line_runs.iter().cloned());
        }
        let left_indent_pt = if align == Alignment::Left {
            if first_line_indent_pt > 0.0 {
                body_x0 - x0
            } else {
                indent
            }
        } else {
            0.0
        };
        paragraphs.push(Paragraph {
            runs,
            alignment: align,
            left_indent_pt,
            first_line_indent_pt,
            line_spacing: LineSpacingRule::ExactPt(line_h),
            source_bbox: RectPt::new(
                block.lines[i].x0,
                block.lines[i].baseline - size,
                block.x1(),
                block.lines[j - 1].baseline + 0.25 * size,
            ),
            ..Default::default()
        });
        i = j;
    }

    // Width: the widest line plus room so a slightly wider substitute never wraps a line.
    let slack = if any_substitute {
        0.10 * widest + 6.0
    } else {
        0.02 * widest + 4.0
    };
    let text_w = widest + slack;
    let box_w = text_w + 2.0 * TEXTBOX_X_INSET_PT;
    let x_left = match align {
        Alignment::Left | Alignment::Justify => x0 - TEXTBOX_X_INSET_PT,
        Alignment::Right => block.x1() - TEXTBOX_X_INSET_PT - box_w,
        Alignment::Center => 0.5 * (x0 + block.x1()) - 0.5 * text_w - TEXTBOX_X_INSET_PT,
    };
    let box_h = line_h * block.lines.len() as f32;
    TextBoxBlock {
        bbox_pt: RectPt::new(x_left, top, x_left + box_w, top + box_h),
        paragraphs,
        fill_color: None,
        border: None,
        auto_fit_height: true,
    }
}

/// A line of text that can be edited in place on the PDF (see `p2d_pdf::edit`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditableLine {
    /// Position of the line in the page's list.
    pub id: usize,
    /// Indices into `PageIr::glyphs` of every glyph the line covers (including glyphs printed twice).
    pub glyph_indices: Vec<usize>,
    /// The line's text, every space the PDF draws included.
    pub text: String,
    /// Visual page points (top-left origin).
    pub bbox: RectPt,
    pub baseline: f32,
    /// Size the line's glyphs mostly appear at.
    pub font_size_pt: f32,
    /// Space added after every character beyond the font's own advance (`Tc`), in points.
    pub char_spacing_pt: f32,
    /// Space added after each space beyond `char_spacing_pt` (`Tw`, justification), in points.
    pub word_spacing_pt: f32,
    /// Horizontal stretch of the glyphs (`Tz`), 1 when none.
    pub horiz_scale: f32,
    /// The edge that stays in place when the text gets longer or shorter.
    pub align: Alignment,
    /// The line's styles in order; their lengths add up to the characters of `text`.
    pub runs: Vec<EditableRun>,
}

/// A stretch of an editable line drawn in one font, size and colour.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditableRun {
    /// Number of characters of the line's `text` in this style.
    pub len: usize,
    /// A glyph of the run (index into `PageIr::glyphs`): retyped text is set in its PDF font.
    pub glyph_index: usize,
    /// A glyph of each other PDF font the run draws with (the same typeface split over several
    /// subsets), for characters the first one lacks.
    #[serde(default)]
    pub other_glyphs: Vec<usize>,
    /// Font of the run in `PageIr::fonts`.
    pub font_idx: usize,
    /// Size the glyphs appear at.
    pub font_size_pt: f32,
    /// Size set in the PDF (differs when the embedded font draws smaller than nominal).
    pub nominal_font_size_pt: f32,
    pub color: RgbaColor,
    /// Installed font that looks like the PDF's, for characters the PDF's font cannot draw.
    pub font_family: String,
    /// Whether `font_family` is the PDF font's own family (same glyphs) rather than a look-alike.
    pub same_family: bool,
    pub bold: bool,
    pub italic: bool,
    pub serif: bool,
    pub monospace: bool,
}

/// The text of an editable line, keeping every space the PDF draws, and its runs of one style:
/// a glyph of each PDF font the run draws with (producers may split one typeface over several
/// subset fonts) and its character count. A space takes the style of what precedes it.
fn line_text_runs<'a>(
    seg: &Segment<'a>,
    fonts: &[FontIr],
    profile: Profile,
    normalize_skia_zwsp: bool,
    fixes: &mut usize,
) -> (String, Vec<(Vec<&'a GlyphIr>, usize)>) {
    let gap_pt = word_gap_pt(seg);
    let mut text = String::new();
    let mut runs: Vec<(Vec<&GlyphIr>, usize)> = Vec::new();
    let style = |g: &GlyphIr| {
        let font = fonts.get(g.font_idx);
        let face = font.map(|f| (f.family_name.as_str(), f.is_bold, f.is_italic));
        (face, (g.font_size_pt * 2.0).round() as i32, g.color)
    };
    let mut prev: Option<&GlyphIr> = None;
    // Spaces before the first run belong to it.
    let mut leading = 0usize;
    for &g in &seg.glyphs {
        let s = normalize_unicode_str(&g.unicode, profile, normalize_skia_zwsp, fixes);
        if s.trim().is_empty() {
            text.push(if s.starts_with('\u{00A0}') { '\u{00A0}' } else { ' ' });
            match runs.last_mut() {
                Some(run) => {
                    run.1 += 1;
                    // The typeface may draw its spaces from another of its subset fonts.
                    if style(run.0[0]) == style(g) && !run.0.iter().any(|o| o.font_idx == g.font_idx) {
                        run.0.push(g);
                    }
                }
                None => leading += 1,
            }
            prev = Some(g);
            continue;
        }
        // A word break drawn as a gap gets a space.
        if let Some(p) = prev
            && !is_space_glyph(p)
            && g.bbox.x0 - glyph_end(p) > gap_pt
            && let Some(run) = runs.last_mut()
        {
            text.push(' ');
            run.1 += 1;
        }
        let n = s.chars().count();
        text.push_str(&s);
        match runs.last_mut() {
            Some(run) if style(run.0[0]) == style(g) => {
                run.1 += n;
                if !run.0.iter().any(|o| o.font_idx == g.font_idx) {
                    run.0.push(g);
                }
            }
            _ => runs.push((vec![g], n + std::mem::take(&mut leading))),
        }
        prev = Some(g);
    }
    (text, runs)
}

fn median(mut values: Vec<f32>) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);
    Some(values[values.len() / 2])
}

fn mean(values: &[f32]) -> Option<f32> {
    (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
}

/// How the PDF spaced a line out: extra space after each character, extra space after each
/// space, and horizontal stretch, measured against the glyphs' own advances. `space_advance` is
/// the width of the font's space, to read word spacing on lines that draw no space glyphs.
///
/// Spacing is averaged rather than taken from the most common gap: some producers round every
/// advance to a device grid (wkhtmltopdf: 0.288 pt), and only the average keeps the line's width.
fn line_spacing(seg: &Segment<'_>, space_advance: Option<f32>) -> (f32, f32, f32) {
    let horiz_scale = median(seg.glyphs.iter().map(|g| g.horiz_scale).collect()).unwrap_or(1.0);
    // Only rounding noise counts as no stretch: the saved line sets the stretch with `Tz` alone.
    let horiz_scale = if (horiz_scale - 1.0).abs() < 0.001 { 1.0 } else { horiz_scale };
    let gap = |a: &GlyphIr, b: &GlyphIr| b.bbox.x0 - glyph_end(a);
    let is_space = |g: &GlyphIr| is_space_glyph(g);
    let letter_gaps: Vec<f32> = seg
        .glyphs
        .windows(2)
        .filter(|w| w[0].font_idx == w[1].font_idx && !is_space(w[0]) && !is_space(w[1]))
        .map(|w| gap(w[0], w[1]))
        .collect();
    // Gaps far from the usual one are word breaks (lines without space glyphs) or kerning
    // outliers, not letter spacing.
    let usual = median(letter_gaps.clone()).unwrap_or(0.0);
    let band = 0.12 * seg.size;
    let core: Vec<f32> = letter_gaps.iter().copied().filter(|g| (g - usual).abs() <= band).collect();
    let char_spacing = if core.len() >= 2 { mean(&core).unwrap_or(0.0) } else { 0.0 };
    let char_spacing = if char_spacing.abs() < 0.002 * seg.size { 0.0 } else { char_spacing };

    // Around a single space: gap before it and after it, beyond the character spacing both get.
    // (Some producers leave the spacing before a space out and put it all after.)
    let space_gaps: Vec<f32> = seg
        .glyphs
        .windows(3)
        .filter(|w| !is_space(w[0]) && is_space(w[1]) && !is_space(w[2]))
        .map(|w| gap(w[0], w[1]) + gap(w[1], w[2]) - 2.0 * char_spacing)
        .filter(|g| g.abs() < 2.0 * seg.size)
        .collect();
    // Word breaks drawn as gaps, whatever the fonts around them: where the editor's text gets a
    // space.
    let word_gap = word_gap_pt(seg);
    let break_gaps: Vec<f32> = seg
        .glyphs
        .windows(2)
        .filter(|w| !is_space(w[0]) && !is_space(w[1]))
        .map(|w| gap(w[0], w[1]))
        .filter(|g| *g > word_gap)
        .collect();
    let word_spacing = if !space_gaps.is_empty() {
        mean(&space_gaps).unwrap_or(0.0)
    } else {
        // Word breaks drawn as gaps: gap = character spacing, space, character spacing, extra.
        match (mean(&break_gaps), space_advance) {
            (Some(b), Some(space)) if b < 2.0 * seg.size => b - space * horiz_scale - 2.0 * char_spacing,
            _ => 0.0,
        }
    };
    let mut word_spacing = if word_spacing.abs() < 0.005 * seg.size { 0.0 } else { word_spacing };
    let mut char_spacing = char_spacing;

    // Retyping the line's own text must give back its exact width: what the averages leave over
    // (kerning, rounding, uneven justification) goes to the word spacing, or in a single word to
    // the letter spacing. Every gap gets the letter spacing, every space the word spacing too. A
    // calibration that would eat more than half a space (heavy kerning) is left out: the words
    // would run together.
    let glyphs = &seg.glyphs;
    let gaps = glyphs.len().saturating_sub(1) as f32;
    let total_gap: f32 = glyphs.windows(2).map(|w| gap(w[0], w[1])).sum();
    let spaces = glyphs.iter().take(glyphs.len().saturating_sub(1)).filter(|g| is_space(g)).count() as f32;
    let breaks = break_gaps.len() as f32;
    let leftover = total_gap - gaps * char_spacing;
    let space_width = mean(&glyphs.iter().filter(|g| is_space(g)).map(|g| g.advance_pt).collect::<Vec<_>>())
        .or(space_advance.map(|s| s * horiz_scale));
    let keeps_space = |ws: f32| space_width.is_none_or(|s| s + char_spacing + ws >= 0.5 * s);
    if spaces > 0.0 {
        let calibrated = leftover / spaces;
        if (calibrated - word_spacing).abs() < 0.25 * seg.size && keeps_space(calibrated) {
            word_spacing = calibrated;
        }
    } else if breaks > 0.0 && let Some(space) = space_advance {
        // A word break drawn as a gap is a typed space: its advance and letter spacing, plus the
        // word spacing.
        let calibrated = leftover / breaks - space * horiz_scale - char_spacing;
        if (calibrated - word_spacing).abs() < 0.25 * seg.size && keeps_space(calibrated) {
            word_spacing = calibrated;
        }
    } else if gaps > 0.0 && breaks == 0.0 {
        let calibrated = total_gap / gaps;
        if (calibrated - char_spacing).abs() < 0.05 * seg.size {
            char_spacing = calibrated;
        }
    }
    (char_spacing, word_spacing, horiz_scale)
}

/// Advance of `font`'s space at `size` points: from its embedded program, else from a space the
/// document draws in it.
fn space_advance(font: Option<&FontIr>, size: f32) -> Option<f32> {
    use skrifa::MetadataProvider;
    use skrifa::instance::{LocationRef, Size};
    let font = font?;
    let from_program = font.program.as_ref().and_then(|program| {
        let face = skrifa::FontRef::from_index(&program.0, 0).ok()?;
        let gid = face.charmap().map(' ')?;
        face.glyph_metrics(Size::new(size), LocationRef::default()).advance_width(gid)
    });
    from_program.or_else(|| {
        let shape = font.shapes.values().find(|s| s.unicode == ' ')?;
        Some(shape.advance * size / 1000.0)
    })
}

/// Which edge of `seg` its neighbours line up with: a column of amounts shares its right edge, a
/// centred title its middle. Left unless the page clearly says otherwise.
fn line_alignment(seg: &Segment<'_>, others: &[Segment<'_>], page_width: f32) -> Alignment {
    let tol = 0.6;
    let (mut left, mut right, mut center) = (0, 0, 0);
    for o in others {
        let near = (o.baseline - seg.baseline).abs() <= 25.0 * seg.size && (o.baseline - seg.baseline).abs() > 0.5;
        if !near || (o.size - seg.size).abs() > 0.3 * seg.size {
            continue;
        }
        let same_x0 = (o.x0 - seg.x0).abs() <= tol;
        let same_x1 = (o.x1 - seg.x1).abs() <= tol;
        let same_mid = ((o.x0 + o.x1) - (seg.x0 + seg.x1)).abs() <= 2.0 * tol;
        if same_x0 && !same_x1 {
            left += 1;
        } else if same_x1 && !same_x0 {
            right += 1;
        } else if same_mid && !same_x0 {
            center += 1;
        }
    }
    // Amounts and numbers sit flush right in tables unless the page says otherwise.
    let numeric = seg.glyphs.iter().any(|g| g.unicode.chars().any(|c| c.is_ascii_digit()))
        && seg.glyphs.iter().all(|g| {
            g.unicode
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_whitespace() || ",.%€$£+-−()/".contains(c))
        });
    // A line alone in the middle of the page is a centred title.
    let page_centred = left == 0 && ((seg.x0 + seg.x1) - page_width).abs() <= 2.0 && seg.x0 > 0.15 * page_width;
    if right > left && right >= center || numeric && left == 0 && center <= right {
        Alignment::Right
    } else if center > left || page_centred {
        Alignment::Center
    } else {
        Alignment::Left
    }
}

/// The page's text as editable lines, top to bottom then left to right.
pub fn editable_lines(page: &PageIr, params: &Params, metrics: &dyn FontMetrics) -> Vec<EditableLine> {
    let has_visible_text = page.glyphs.iter().any(is_visible_glyph);
    let glyphs: Vec<&GlyphIr> = page
        .glyphs
        .iter()
        .filter(|g| has_visible_text && is_visible_glyph(g) && g.angle_deg.abs() <= EXACT_TEXT_MAX_ANGLE_DEG)
        .collect();
    let mut segments: Vec<Segment<'_>> = group_lines(glyphs)
        .into_iter()
        .flat_map(split_segments)
        .flat_map(split_at_jumps)
        .collect();
    segments.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.x0.total_cmp(&b.x0)));
    let page_fonts: Vec<TargetFont> = (0..page.fonts.len())
        .map(|i| target_font(page, i, metrics, params.profile))
        .collect();
    let unit_spaces: Vec<Option<f32>> = (0..page.fonts.len())
        .map(|i| space_advance(page.fonts.get(i), 1.0))
        .collect();
    let mut fixes = 0usize;
    segments
        .iter()
        .filter_map(|seg| {
            let tol = 0.2 * seg.size;
            let glyph_indices: Vec<usize> = page
                .glyphs
                .iter()
                .enumerate()
                .filter(|(_, g)| {
                    // Spaces right after the line belong to it too, when set like it: they go when
                    // it is retyped.
                    let trailing_space = is_space_glyph(g)
                        && (g.font_size_pt - seg.size).abs() <= 0.3 * seg.size
                        && (g.baseline_y - seg.baseline).abs() <= 0.2 * g.font_size_pt.min(seg.size);
                    let end = if trailing_space { seg.x1 + 0.6 * seg.size } else { seg.x1 - 0.1 };
                    (g.baseline_y - seg.baseline).abs() <= tol
                        && g.bbox.x0 >= seg.x0 - 0.1
                        && g.bbox.x0 < end
                        && g.angle_deg.abs() <= EXACT_TEXT_MAX_ANGLE_DEG
                })
                .map(|(i, _)| i)
                .collect();
            let (text, run_glyphs) =
                line_text_runs(seg, &page.fonts, params.profile, params.normalize_skia_zwsp, &mut fixes);
            // Nothing but characters the normalization blanks: nothing to edit.
            if run_glyphs.is_empty() {
                return None;
            }
            let index_of = |g: &GlyphIr| {
                glyph_indices
                    .iter()
                    .copied()
                    .find(|&i| std::ptr::eq(&page.glyphs[i], g))
                    .unwrap_or_default()
            };
            let runs: Vec<EditableRun> = run_glyphs
                .iter()
                .map(|(glyphs, len)| {
                    let g = glyphs[0];
                    let font = page.fonts.get(g.font_idx);
                    let tf = page_fonts
                        .get(g.font_idx)
                        .cloned()
                        .unwrap_or_else(|| target_font(page, g.font_idx, metrics, params.profile));
                    EditableRun {
                        len: *len,
                        glyph_index: index_of(g),
                        other_glyphs: glyphs[1..].iter().map(|o| index_of(o)).collect(),
                        font_idx: g.font_idx,
                        font_size_pt: g.font_size_pt,
                        nominal_font_size_pt: nominal_size(font, g.font_size_pt),
                        color: g.color,
                        font_family: tf.family,
                        same_family: tf.same_family,
                        bold: tf.bold,
                        italic: tf.italic,
                        serif: font.is_some_and(|f| f.is_serif),
                        monospace: font.is_some_and(|f| f.is_monospace),
                    }
                })
                .collect();
            let first = seg.glyphs[0];
            let font = page.fonts.get(first.font_idx);
            let space = unit_spaces
                .get(first.font_idx)
                .and_then(|u| *u)
                .map(|u| u * nominal_size(font, seg.size));
            let (char_spacing_pt, word_spacing_pt, horiz_scale) = line_spacing(seg, space);
            Some(EditableLine {
                id: 0,
                glyph_indices,
                text,
                bbox: RectPt::new(
                    seg.x0,
                    seg.baseline - 0.8 * seg.size,
                    seg.x1,
                    seg.baseline + 0.25 * seg.size,
                ),
                baseline: seg.baseline,
                font_size_pt: seg.size,
                char_spacing_pt,
                word_spacing_pt,
                horiz_scale,
                align: line_alignment(seg, &segments, page.width_pt),
                runs,
            })
        })
        .enumerate()
        .map(|(id, line)| EditableLine { id, ..line })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::NoFontMetrics;

    fn glyph(ch: &str, x: f32, baseline: f32, advance: f32) -> GlyphIr {
        GlyphIr {
            unicode: ch.to_string(),
            glyph_id: 0,
            bbox: RectPt::new(x, baseline - 9.8, x + advance, baseline + 2.6),
            baseline_y: baseline,
            font_size_pt: 12.0,
            advance_pt: advance,
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
        }
    }

    /// Glyphs of `text` from `x`, every character `advance` wide, spaces `space` wide.
    fn line(text: &str, x: f32, baseline: f32, advance: f32, space: f32) -> Vec<GlyphIr> {
        let mut out = Vec::new();
        let mut pen = x;
        for c in text.chars() {
            let w = if c == ' ' { space } else { advance };
            out.push(glyph(&c.to_string(), pen, baseline, w));
            pen += w;
        }
        out
    }

    fn page(glyphs: Vec<GlyphIr>) -> DocIr {
        DocIr {
            pages: vec![PageIr {
                width_pt: 595.0,
                height_pt: 842.0,
                fonts: vec![FontIr {
                    postscript_name: "Helvetica".into(),
                    family_name: "Helvetica".into(),
                    ..Default::default()
                }],
                glyphs,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_text_box_follows_the_pages_baseline_model() {
        let mut glyphs = line("AB CD", 72.0, 100.0, 8.0, 3.336);
        glyphs.extend(line("EF", 72.0, 114.0, 8.0, 3.336));
        let (doc, _) = reconstruct_exact(&page(glyphs), &Params::default(), &NoFontMetrics);
        let FlowBlock::TextBox(tb) = &doc.sections[0].blocks[0] else {
            panic!("expected a text box, got {:?}", doc.sections[0].blocks);
        };
        assert!((tb.bbox_pt.x0 - (72.0 - TEXTBOX_X_INSET_PT)).abs() < 1e-3);
        assert!((tb.bbox_pt.y0 - (100.0 - EXACT_BASELINE_RATIO * 14.0)).abs() < 1e-3);
        assert_eq!(tb.paragraphs.len(), 1);
        assert_eq!(tb.paragraphs[0].line_spacing, LineSpacingRule::ExactPt(14.0));
        let text: String = tb.paragraphs[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "AB CD\nEF");
    }

    #[test]
    fn test_stretched_space_gets_the_difference_as_tracking() {
        // A justified line: the space glyph is 3.336 wide but the next word starts 6 pt later.
        let mut glyphs = line("AB", 72.0, 100.0, 8.0, 3.336);
        glyphs.push(glyph(" ", 88.0, 100.0, 3.336));
        glyphs.extend(line("CD", 94.0, 100.0, 8.0, 3.336));
        let (doc, _) = reconstruct_exact(&page(glyphs), &Params::default(), &NoFontMetrics);
        let FlowBlock::TextBox(tb) = &doc.sections[0].blocks[0] else {
            panic!("expected a text box");
        };
        let space = tb.paragraphs[0].runs.iter().find(|r| r.text == " ").expect("space run");
        assert!(
            (space.style.char_tracking_pt - (6.0 - 3.336)).abs() < 0.05,
            "{}",
            space.style.char_tracking_pt
        );
        assert!(
            tb.paragraphs[0]
                .runs
                .iter()
                .filter(|r| r.text != " ")
                .all(|r| r.style.char_tracking_pt == 0.0)
        );
    }

    #[test]
    fn test_editable_lines_cover_every_glyph_once() {
        let mut glyphs = line("Bonjour", 72.0, 100.0, 7.0, 3.3);
        glyphs.extend(line("Total", 300.0, 100.0, 7.0, 3.3));
        let doc = page(glyphs);
        let lines = editable_lines(&doc.pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(
            lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            vec!["Bonjour", "Total"]
        );
        let mut covered: Vec<usize> = lines.iter().flat_map(|l| l.glyph_indices.clone()).collect();
        covered.sort_unstable();
        assert_eq!(covered, (0..12).collect::<Vec<_>>());
    }

    /// Like [`line`], with `tracking` after every character and `extra` more after every space.
    fn spaced_line(text: &str, x: f32, baseline: f32, tracking: f32, extra: f32) -> Vec<GlyphIr> {
        let mut out = Vec::new();
        let mut pen = x;
        for c in text.chars() {
            let w = if c == ' ' { 3.0 } else { 6.0 };
            out.push(glyph(&c.to_string(), pen, baseline, w));
            pen += w + tracking + if c == ' ' { extra } else { 0.0 };
        }
        out
    }

    fn texts(lines: &[EditableLine]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn test_editable_line_measures_letter_and_word_spacing() {
        let doc = page(spaced_line("Le nom du client", 72.0, 100.0, 0.4, 2.0));
        let lines = editable_lines(&doc.pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(texts(&lines), vec!["Le nom du client"]);
        assert!((lines[0].char_spacing_pt - 0.4).abs() < 0.01, "{}", lines[0].char_spacing_pt);
        assert!((lines[0].word_spacing_pt - 2.0).abs() < 0.01, "{}", lines[0].word_spacing_pt);
        assert_eq!(lines[0].horiz_scale, 1.0);
    }

    #[test]
    fn test_editable_lines_split_at_form_padding_and_column_gaps() {
        // A label padded with spaces before its value, and a table row whose two cells are only
        // 11 pt apart (closer than a column break for the conversion).
        let mut glyphs = line("Nom :    Dupont", 72.0, 100.0, 6.0, 3.0);
        glyphs.extend(line("20,0 %", 300.0, 140.0, 6.0, 3.0));
        glyphs.extend(line("500,00 €", 344.0, 140.0, 6.0, 3.0));
        let lines = editable_lines(&page(glyphs).pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(texts(&lines), vec!["Nom :", "Dupont", "20,0 %", "500,00 €"]);
    }

    #[test]
    fn test_editable_text_keeps_every_space() {
        let lines = editable_lines(&page(line("A  B", 72.0, 100.0, 6.0, 3.0)).pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(texts(&lines), vec!["A  B"]);
    }

    #[test]
    fn test_amount_columns_are_right_aligned() {
        let mut glyphs = line("Total", 72.0, 100.0, 6.0, 3.0);
        // Two amounts sharing their right edge (445), and a lone amount with no neighbour.
        glyphs.extend(line("1 250,00", 400.0, 100.0, 6.0, 3.0));
        glyphs.extend(line("90,00", 415.0, 120.0, 6.0, 3.0));
        glyphs.extend(line("12,50", 200.0, 300.0, 6.0, 3.0));
        let lines = editable_lines(&page(glyphs).pages[0], &Params::default(), &NoFontMetrics);
        let align = |t: &str| lines.iter().find(|l| l.text == t).map(|l| l.align);
        assert_eq!(align("Total"), Some(Alignment::Left));
        assert_eq!(align("1 250,00"), Some(Alignment::Right));
        assert_eq!(align("90,00"), Some(Alignment::Right));
        assert_eq!(align("12,50"), Some(Alignment::Right));
    }

    #[test]
    fn test_heavy_kerning_does_not_eat_the_spaces() {
        // "AVxx" three times: A-V kerned 1.8 pt tighter, every other gap 0, spaces 3 pt.
        let mut glyphs = Vec::new();
        let mut pen = 72.0;
        for word in 0..3 {
            for (c, after) in [("A", -1.8), ("V", 0.0), ("x", 0.0), ("x", 0.0)] {
                glyphs.push(glyph(c, pen, 100.0, 6.0));
                pen += 6.0 + after;
            }
            if word < 2 {
                glyphs.push(glyph(" ", pen, 100.0, 3.0));
                pen += 3.0;
            }
        }
        let lines = editable_lines(&page(glyphs).pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(texts(&lines), vec!["AVxx AVxx AVxx"]);
        assert!(lines[0].word_spacing_pt.abs() < 0.01, "{}", lines[0].word_spacing_pt);
    }

    #[test]
    fn test_smaller_spaces_nearby_stay_with_their_own_line() {
        let mut glyphs = line("Hello", 72.0, 100.0, 6.0, 3.0);
        let mut small = glyph(" ", 102.5, 101.5, 1.5);
        small.font_size_pt = 6.0;
        glyphs.push(small);
        let lines = editable_lines(&page(glyphs).pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(lines[0].glyph_indices, (0..5).collect::<Vec<_>>());
    }

    #[test]
    fn test_word_break_at_a_font_change_counts_as_a_space() {
        // No space glyphs: "AB", "CD" and "EF" 2.5 pt apart, "EF" from a second subset font.
        let mut glyphs = line("AB", 72.0, 100.0, 6.0, 3.0);
        glyphs.extend(line("CD", 86.5, 100.0, 6.0, 3.0));
        let mut ef = line("EF", 101.0, 100.0, 6.0, 3.0);
        for g in &mut ef {
            g.font_idx = 1;
        }
        glyphs.extend(ef);
        let mut doc = page(glyphs);
        let space = GlyphShape {
            unicode: ' ',
            advance: 278.0,
            path: Vec::new(),
        };
        doc.pages[0].fonts[0].shapes.insert(3, space);
        let second = doc.pages[0].fonts[0].clone();
        doc.pages[0].fonts.push(second);
        let lines = editable_lines(&doc.pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(texts(&lines), vec!["AB CD EF"]);
        // Each break is a 3.336 pt space and the word spacing: 2.5 - 3.336.
        assert!((lines[0].word_spacing_pt + 0.836).abs() < 0.01, "{}", lines[0].word_spacing_pt);
    }

    #[test]
    fn test_runs_cover_the_whole_text() {
        // A zero-width space (Skia's space) before the text.
        let mut glyphs = vec![glyph("\u{200B}", 66.0, 100.0, 6.0)];
        glyphs.extend(line("AB", 72.0, 100.0, 6.0, 3.0));
        let params = Params {
            normalize_skia_zwsp: true,
            ..Params::default()
        };
        let lines = editable_lines(&page(glyphs).pages[0], &params, &NoFontMetrics);
        for l in &lines {
            assert_eq!(l.runs.iter().map(|r| r.len).sum::<usize>(), l.text.chars().count(), "{:?}", l.text);
        }
    }

    #[test]
    fn test_slight_stretch_is_kept() {
        let mut glyphs = line("Bonjour", 72.0, 100.0, 6.0, 3.0);
        for g in &mut glyphs {
            g.horiz_scale = 0.992;
        }
        let lines = editable_lines(&page(glyphs).pages[0], &Params::default(), &NoFontMetrics);
        assert!((lines[0].horiz_scale - 0.992).abs() < 1e-4, "{}", lines[0].horiz_scale);
    }

    #[test]
    fn test_c1_control_glyphs_are_text() {
        // A broken ToUnicode maps a visible glyph to U+0095: the normalization keeps it.
        let mut glyphs = vec![glyph("\u{0095}", 66.0, 100.0, 6.0)];
        glyphs.extend(line("AB", 72.0, 100.0, 6.0, 3.0));
        let lines = editable_lines(&page(glyphs).pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(texts(&lines), vec!["\u{0095}AB"]);
        assert_eq!(lines[0].glyph_indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_editable_runs_follow_style_changes() {
        // "Date : " in bold, then the value in regular.
        let mut glyphs = line("Date : ", 72.0, 100.0, 6.0, 3.0);
        let mut value = line("12/03", 72.0 + 6.0 * 5.0 + 3.0 * 2.0, 100.0, 6.0, 3.0);
        for g in &mut value {
            g.font_idx = 1;
        }
        glyphs.extend(value);
        let mut doc = page(glyphs);
        doc.pages[0].fonts.push(FontIr {
            postscript_name: "Helvetica-Bold".into(),
            family_name: "Helvetica".into(),
            is_bold: true,
            ..Default::default()
        });
        let lines = editable_lines(&doc.pages[0], &Params::default(), &NoFontMetrics);
        assert_eq!(texts(&lines), vec!["Date : 12/03"]);
        let runs: Vec<(usize, usize, bool)> = lines[0].runs.iter().map(|r| (r.len, r.font_idx, r.bold)).collect();
        assert_eq!(runs, vec![(7, 0, false), (5, 1, true)]);
    }
}
