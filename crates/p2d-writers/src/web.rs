use p2d_ir::*;
use crate::docx::xml_escape;

/// Serialize `FlowDoc` to clean, layout-faithful HTML5 (used both for HTML export and the Visual App live preview).
pub fn write_html(doc: &FlowDoc) -> String {
    let title = doc.title.as_deref().unwrap_or("Document converti");
    let mut out = format!(
        r#"<!DOCTYPE html>
<html lang="fr">
<head>
<meta charset="utf-8">
<title>{}</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Montserrat:ital,wght@0,400;0,600;0,700;1,400;1,700&family=Nunito:ital,wght@0,400;0,700;1,400&family=Oswald:wght@400;600;700&display=swap">
<style>
  * {{ box-sizing: border-box; }}
  body {{
    font-family: "Helvetica Neue", Arial, sans-serif;
    color: #18181b;
    line-height: 1.24;
    margin: 0;
    padding: 18px 12px;
    background: #eef2f6;
  }}
  .page-sheet {{
    position: relative;
    background: #ffffff;
    max-width: 100%;
    margin: 0 auto 24px auto;
    box-shadow: 0 6px 24px rgba(15, 23, 42, 0.09);
    border: 1px solid #cbd5e1;
    border-radius: 6px;
  }}
  .page-badge {{
    position: absolute;
    top: 8px;
    right: 12px;
    font-size: 8.5pt;
    font-weight: 600;
    color: #64748b;
    background: #f1f5f9;
    border: 1px solid #e2e8f0;
    border-radius: 999px;
    padding: 2px 9px;
    letter-spacing: 0.02em;
  }}
  h1, h2, p {{ margin: 0; padding: 0; }}
  table {{ border-collapse: collapse; width: 100%; }}
  td, th {{ border: 1px solid #94a3b8; padding: 2pt 3.5pt; vertical-align: top; }}
  th {{ font-weight: 700; text-align: left; white-space: nowrap; }}
  table.layout-grid, table.layout-grid > tbody > tr > td, table.layout-grid tr > td {{
    border: none;
    padding: 0.5pt 8pt 0.5pt 0;
  }}
  table.layout-grid tr > td:last-child {{
    padding-right: 0;
  }}
  table.banner-box, table.banner-box tr > td {{
    border: none;
    padding: 7pt 12pt;
    border-radius: 3px;
  }}
  p.list-bullet::before {{ content: "• "; font-weight: bold; color: #1a5fb4; }}
  p.list-bullet {{ padding-left: 14pt; }}
  p.tabbed-row {{
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8pt;
  }}
  p.tabbed-row > span:last-child {{
    white-space: nowrap;
    text-align: right;
  }}
  p.tabbed-dot {{
    border-bottom: 1px dotted #cbd5e1;
    padding-bottom: 2px;
  }}
  .doc-header {{
    font-size: 9.5pt;
    color: #475569;
    border-bottom: 1px solid #e2e8f0;
    padding-bottom: 6px;
    margin-bottom: 12px;
  }}
  .doc-footer {{
    font-size: 9pt;
    color: #64748b;
    border-top: 1px solid #e2e8f0;
    padding-top: 6px;
    margin-top: 18px;
    text-align: center;
  }}
</style>
</head>
<body>
"#,
        xml_escape(title)
    );

    let total_pages = doc.sections.len().max(1);
    for (idx, sec) in doc.sections.iter().enumerate() {
        if is_positioned_section(sec) {
            write_positioned_page(&mut out, sec, idx, total_pages);
            continue;
        }
        let mt = sec.margin_top_pt.clamp(12.0, 120.0);
        let mb = sec.margin_bottom_pt.clamp(12.0, 90.0);
        let ml = sec.margin_left_pt.clamp(12.0, 90.0);
        let mr = sec.margin_right_pt.clamp(12.0, 90.0);
        let pw = sec.page_width_pt.clamp(300.0, 1200.0);
        let ph = sec.page_height_pt.clamp(300.0, 1400.0);
        let content_w = (pw - ml - mr).max(180.0);
        out.push_str(&format!(
            r#"<div class="page-sheet" style="width:{pw:.1}pt;min-height:{ph:.1}pt;padding:{mt:.1}pt {mr:.1}pt {mb:.1}pt {ml:.1}pt;"><div class="page-badge">Page {} / {}</div>"#,
            idx + 1,
            total_pages
        ));

        if !sec.header_paragraphs.is_empty() {
            out.push_str(r#"<div class="doc-header">"#);
            for p in &sec.header_paragraphs {
                write_html_paragraph(&mut out, p);
            }
            out.push_str("</div>\n");
        }
        let mut active_float_bottom_y: Option<f32> = None;
        let mut prev_block_y1: Option<f32> = None;
        let mut bi = 0;
        while bi < sec.blocks.len() {
            let block = &sec.blocks[bi];
            if let Some(float_y1) = active_float_bottom_y {
                let block_y0 = match block {
                    FlowBlock::Paragraph(p) => Some(p.source_bbox.y0),
                    FlowBlock::Table(t) => Some(t.source_bbox.y0),
                    FlowBlock::Image(i) => Some(i.bbox_pt.y0),
                    FlowBlock::TextBox(tb) => Some(tb.bbox_pt.y0),
                    FlowBlock::Shape(sh) => Some(sh.bbox_pt.y0),
                    FlowBlock::PageBreak => None,
                };
                if block_y0.is_some_and(|y0| y0 >= float_y1 - 8.0) {
                    out.push_str(r#"<div style="clear:both;"></div>"#);
                    active_float_bottom_y = None;
                }
            }
            match block {
                FlowBlock::Paragraph(p) => {
                    write_html_paragraph(&mut out, p);
                    prev_block_y1 = Some(p.source_bbox.y1);
                }
                FlowBlock::Table(t) => {
                    prev_block_y1 = Some(t.source_bbox.y1);
                    let is_h_rule = t.is_borderless_layout
                        && t.rows.len() == 1
                        && t.col_widths_pt.len() == 1
                        && t.rows[0].cells.first().is_some_and(|c| {
                            c.border_bottom.is_some()
                                && c.paragraphs.iter().all(|p| p.runs.is_empty())
                        });
                    if is_h_rule {
                        if let Some(bdr) = t.rows[0].cells[0].border_bottom {
                            let w_pt = bdr.width_pt.clamp(0.75, 3.0);
                            let hex = bdr.color.to_hex_rgb();
                            let before = t.space_before_pt.max(2.0);
                            let after = t.space_after_pt.max(2.0);
                            out.push_str(&format!(
                                r#"<hr style="border:0;border-top:{w_pt:.1}pt solid #{hex};margin:{before:.1}pt 0 {after:.1}pt 0;" />"#
                            ));
                            out.push('\n');
                            bi += 1;
                            continue;
                        }
                    }

                    let is_banner = t.is_borderless_layout
                        && t.col_widths_pt.len() == 1
                        && t.rows
                            .first()
                            .and_then(|r| r.cells.first())
                            .is_some_and(|c| c.shading_color.is_some());
                    let tbl_class = if is_banner {
                        r#" class="banner-box""#
                    } else if t.is_borderless_layout {
                        r#" class="layout-grid""#
                    } else {
                        ""
                    };
                    let total_w: f32 = t.col_widths_pt.iter().sum::<f32>().max(1.0);
                    let mut tbl_styles = vec![
                        format!("margin-top:{:.1}pt", t.space_before_pt),
                        format!("margin-bottom:{:.1}pt", t.space_after_pt),
                    ];
                    if t.left_indent_pt >= 3.0 {
                        tbl_styles.push(format!("margin-left:{:.1}pt", t.left_indent_pt));
                    }
                    if (!t.is_borderless_layout && total_w < content_w * 0.90)
                        || t.left_indent_pt > 20.0
                    {
                        tbl_styles.push(format!("width:{total_w:.1}pt;max-width:100%"));
                    }
                    let tbl_margin = format!(r#" style="{}""#, tbl_styles.join(";"));
                    out.push_str(&format!("<table{tbl_class}{tbl_margin}>\n"));
                    for row in &t.rows {
                        if !t.is_borderless_layout && row.min_height_pt >= 24.0 {
                            out.push_str(&format!(r#"  <tr style="height:{:.1}pt">"#, row.min_height_pt));
                            out.push('\n');
                        } else {
                            out.push_str("  <tr>\n");
                        }
                        let tag = if row.is_header_row && !t.is_borderless_layout {
                            "th"
                        } else {
                            "td"
                        };
                        for cell in &row.cells {
                            let pct = ((cell.width_pt / total_w) * 100.0).clamp(3.0, 100.0);
                            let mut styles = vec![format!("width:{pct:.1}%")];
                            if let Some(c) = cell.shading_color {
                                styles.push(format!("background-color:#{}", c.to_hex_rgb()));
                                if t.is_borderless_layout && !is_banner {
                                    styles.push("padding:4pt 8pt;border-radius:3px".to_string());
                                }
                            }
                            if t.is_borderless_layout {
                                if let Some(b) = cell.border_top {
                                    styles.push(format!(
                                        "border:1px solid #{};padding:4pt 8pt",
                                        b.color.to_hex_rgb()
                                    ));
                                }
                            } else {
                                let any_bdr = cell
                                    .border_left
                                    .or(cell.border_right)
                                    .or(cell.border_top)
                                    .or(cell.border_bottom);
                                if let Some(b) = any_bdr {
                                    if b.width_pt >= 2.0
                                        && t.col_widths_pt.len() == 1
                                        && t.rows.len() == 1
                                    {
                                        styles.push(format!(
                                            "border:{:.1}pt solid #{};box-shadow:6pt 6pt 0 #{};padding:10pt 14pt",
                                            b.width_pt,
                                            b.color.to_hex_rgb(),
                                            b.color.to_hex_rgb()
                                        ));
                                    } else if t.col_widths_pt.len() == 1
                                        && t.rows.len() == 1
                                        && row.min_height_pt >= 45.0
                                    {
                                        styles.push(format!(
                                            "border-color:#{};padding:8pt 10pt",
                                            b.color.to_hex_rgb()
                                        ));
                                    } else {
                                        styles.push(format!("border-color:#{}", b.color.to_hex_rgb()));
                                    }
                                }
                                if cell.border_top.is_none() {
                                    styles.push("border-top:none".to_string());
                                }
                                if cell.border_bottom.is_none() {
                                    styles.push("border-bottom:none".to_string());
                                }
                                if cell.border_left.is_none() {
                                    styles.push("border-left:none".to_string());
                                }
                                if cell.border_right.is_none() {
                                    styles.push("border-right:none".to_string());
                                }
                            }
                            let style_attr = format!(r#" style="{}""#, styles.join(";"));
                            out.push_str(&format!("    <{tag}{style_attr}>"));
                            for p in &cell.paragraphs {
                                write_html_paragraph(&mut out, p);
                            }
                            out.push_str(&format!("</{tag}>\n"));
                        }
                        out.push_str("  </tr>\n");
                    }
                    out.push_str("</table>\n");
                }
                FlowBlock::TextBox(tb) => {
                    out.push_str(r#"<div class="textbox">"#);
                    for p in &tb.paragraphs {
                        write_html_paragraph(&mut out, p);
                    }
                    out.push_str("</div>\n");
                }
                FlowBlock::Image(img) => {
                    if !img.data.is_empty() {
                        let mime = if img.format == "png" {
                            "image/png"
                        } else {
                            "image/jpeg"
                        };
                        let b64 = encode_base64(&img.data);
                        let w_pt = img.bbox_pt.width().clamp(24.0, 520.0);
                        if !img.inline && !img.behind_doc {
                            let float_dir = if img.bbox_pt.center_x() < sec.page_width_pt * 0.5 {
                                "float:left;margin:2pt 18pt 8pt 0;"
                            } else {
                                "float:right;margin:2pt 0 8pt 18pt;"
                            };
                            active_float_bottom_y = Some(img.bbox_pt.y1);
                            out.push_str(&format!(
                                r#"<div style="{float_dir}"><img src="data:{mime};base64,{b64}" alt="Illustration" style="max-width:100%;width:{w_pt:.1}pt;height:auto;display:block;border-radius:4px;" /></div>"#
                            ));
                        } else {
                            // Check if next block is also a small inline image on the same horizontal row (e.g. CONFORME + SÉCURISÉ badges)
                            if let Some(FlowBlock::Image(next_img)) = sec.blocks.get(bi + 1) {
                                if next_img.inline
                                    && !next_img.data.is_empty()
                                    && (next_img.bbox_pt.center_y() - img.bbox_pt.center_y()).abs() <= 14.0
                                {
                                    let next_mime = if next_img.format == "png" {
                                        "image/png"
                                    } else {
                                        "image/jpeg"
                                    };
                                    let next_b64 = encode_base64(&next_img.data);
                                    let next_w = next_img.bbox_pt.width().clamp(24.0, 260.0);
                                    let top_gap = prev_block_y1
                                        .map(|py1| (img.bbox_pt.y0 - py1 - 1.5).clamp(2.0, 120.0))
                                        .unwrap_or(2.0);
                                    let left_off = (img.bbox_pt.x0.min(next_img.bbox_pt.x0) - ml)
                                        .clamp(0.0, content_w * 0.80);
                                    out.push_str(&format!(
                                        r#"<div style="margin:{top_gap:.1}pt 0 2pt {left_off:.1}pt;"><img src="data:{mime};base64,{b64}" alt="Illustration" style="width:{w_pt:.1}pt;height:auto;display:inline-block;vertical-align:middle;margin-right:8pt;" /><img src="data:{next_mime};base64,{next_b64}" alt="Illustration" style="width:{next_w:.1}pt;height:auto;display:inline-block;vertical-align:middle;" /></div>"#
                                    ));
                                    prev_block_y1 = Some(img.bbox_pt.y1.max(next_img.bbox_pt.y1));
                                    bi += 2;
                                    continue;
                                }
                            }
                            let top_gap = prev_block_y1
                                .map(|py1| (img.bbox_pt.y0 - py1 - 1.5).clamp(2.0, 160.0))
                                .unwrap_or(2.0);
                            let img_align_margin =
                                if (img.bbox_pt.center_x() - sec.page_width_pt * 0.5).abs() <= 35.0
                                {
                                    format!("margin:{top_gap:.1}pt auto 2pt auto;")
                                } else if img.bbox_pt.x0 > sec.margin_left_pt + 18.0 {
                                    let offset = (img.bbox_pt.x0 - sec.margin_left_pt)
                                        .clamp(0.0, sec.page_width_pt * 0.65);
                                    format!("margin:{top_gap:.1}pt 0 2pt {offset:.1}pt;")
                                } else {
                                    format!("margin:{top_gap:.1}pt 0 2pt 0;")
                                };
                            out.push_str(&format!(
                                r#"<div><img src="data:{mime};base64,{b64}" alt="Illustration" style="max-width:100%;width:{w_pt:.1}pt;height:auto;display:block;border-radius:4px;{img_align_margin}" /></div>"#
                            ));
                            prev_block_y1 = Some(img.bbox_pt.y1);
                        }
                    }
                }
                FlowBlock::Shape(_) => {}
                FlowBlock::PageBreak => out.push_str("<hr/>\n"),
            }
            bi += 1;
        }
        if active_float_bottom_y.is_some() {
            out.push_str(r#"<div style="clear:both;"></div>"#);
        }
        if !sec.footer_paragraphs.is_empty() {
            out.push_str(r#"<div class="doc-footer">"#);
            for p in &sec.footer_paragraphs {
                write_html_paragraph(&mut out, p);
            }
            out.push_str("</div>\n");
        }
        out.push_str("</div>\n");
    }

    if doc.sections.iter().any(is_positioned_section) {
        // Positioned pages have a fixed size: zoom them down to the width of the window.
        out.push_str(
            r#"<script>(function(){function fit(){var w=document.documentElement.clientWidth-26;document.querySelectorAll('[data-width-pt]').forEach(function(p){var px=parseFloat(p.dataset.widthPt)*4/3;p.style.zoom=px>w?(w/px).toFixed(4):'';});}addEventListener('resize',fit);fit();})();</script>"#,
        );
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// Sections laid out with absolute positions (`Preset::Exact`): text boxes, shapes, page backgrounds.
fn is_positioned_section(sec: &Section) -> bool {
    sec.blocks.iter().any(|b| {
        matches!(b, FlowBlock::TextBox(_) | FlowBlock::Shape(_))
            || matches!(b, FlowBlock::Image(img) if img.behind_doc && !img.inline)
    })
}

/// A page whose blocks all sit at absolute positions, drawn in the same order as in the DOCX.
fn write_positioned_page(out: &mut String, sec: &Section, idx: usize, total_pages: usize) {
    let (pw, ph) = (sec.page_width_pt, sec.page_height_pt);
    out.push_str(&format!(
        r#"<div class="page-sheet" data-width-pt="{pw:.2}" style="width:{pw:.2}pt;height:{ph:.2}pt;padding:0;overflow:hidden;border-radius:0;"><div class="page-badge" style="z-index:5">Page {} / {}</div>"#,
        idx + 1,
        total_pages
    ));
    for block in &sec.blocks {
        match block {
            FlowBlock::Image(img) if !img.data.is_empty() => {
                let mime = if img.format == "png" { "image/png" } else { "image/jpeg" };
                let b = img.bbox_pt;
                out.push_str(&format!(
                    r#"<img src="data:{mime};base64,{}" alt="" style="position:absolute;left:{:.2}pt;top:{:.2}pt;width:{:.2}pt;height:{:.2}pt;" />"#,
                    encode_base64(&img.data),
                    b.x0,
                    b.y0,
                    b.width(),
                    b.height()
                ));
            }
            FlowBlock::Shape(sh) => {
                // CSS borders sit inside the box while PDF strokes are centred on the outline.
                let half = sh.stroke.map_or(0.0, |s| s.width_pt / 2.0);
                let b = sh.bbox_pt;
                let mut style = format!(
                    "position:absolute;box-sizing:border-box;left:{:.2}pt;top:{:.2}pt;width:{:.2}pt;height:{:.2}pt;",
                    b.x0 - half,
                    b.y0 - half,
                    b.width() + 2.0 * half,
                    b.height() + 2.0 * half
                );
                if let Some(c) = sh.fill_color {
                    style.push_str(&format!("background:{};", css_rgba(c)));
                }
                if let Some(s) = sh.stroke {
                    style.push_str(&format!("border:{:.2}pt solid {};", s.width_pt, css_rgba(s.color)));
                }
                out.push_str(&format!(r#"<div style="{style}"></div>"#));
            }
            FlowBlock::TextBox(tb) => {
                // Pages puts the first baseline 0.8 × line height below the box top; a browser
                // centres the font's ascent + descent in the line instead, so shift by the gap.
                let first = tb.paragraphs.first();
                let (line_h, size) = first
                    .map(|p| {
                        let size = p.runs.first().map_or(11.0, |r| r.style.font_size_pt);
                        match p.line_spacing {
                            LineSpacingRule::ExactPt(l) | LineSpacingRule::AtLeastPt(l) => (l, size),
                            LineSpacingRule::Multiple(m) => (m * size * 1.2, size),
                        }
                    })
                    .unwrap_or((13.2, 11.0));
                let shift = 0.3 * line_h - 0.27 * size;
                let b = tb.bbox_pt;
                out.push_str(&format!(
                    r#"<div style="position:absolute;left:{:.2}pt;top:{:.2}pt;width:{:.2}pt;padding:0 0.5pt;">"#,
                    b.x0,
                    b.y0 + shift,
                    b.width()
                ));
                for p in &tb.paragraphs {
                    write_positioned_paragraph(out, p);
                }
                out.push_str("</div>");
            }
            _ => {}
        }
    }
    out.push_str("</div>\n");
}

fn write_positioned_paragraph(out: &mut String, p: &Paragraph) {
    let line_h = match p.line_spacing {
        LineSpacingRule::ExactPt(l) | LineSpacingRule::AtLeastPt(l) => format!("{l:.2}pt"),
        LineSpacingRule::Multiple(m) => format!("{m:.2}"),
    };
    let align = match p.alignment {
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
        Alignment::Justify => "justify",
    };
    out.push_str(&format!(
        r#"<p style="margin:0;white-space:pre;line-height:{line_h};text-align:{align};padding-left:{:.2}pt;text-indent:{:.2}pt;">"#,
        p.left_indent_pt, p.first_line_indent_pt
    ));
    for r in &p.runs {
        let s = &r.style;
        let mut style = format!(
            "font-family:'{}',{};font-size:{:.2}pt;color:{};",
            xml_escape(&s.font_family),
            if s.font_family.contains("Times") || s.font_family == "Georgia" { "serif" } else { "sans-serif" },
            s.font_size_pt,
            css_rgba(s.color)
        );
        if s.bold {
            style.push_str("font-weight:700;");
        }
        if s.italic {
            style.push_str("font-style:italic;");
        }
        if s.char_tracking_pt.abs() > 0.001 {
            style.push_str(&format!("letter-spacing:{:.3}pt;", s.char_tracking_pt));
        }
        let text = xml_escape(&r.text).replace('\n', "<br>");
        out.push_str(&format!(r#"<span style="{style}">{text}</span>"#));
    }
    out.push_str("</p>");
}

fn css_rgba(c: RgbaColor) -> String {
    if c.a == 255 {
        format!("#{}", c.to_hex_rgb())
    } else {
        format!("rgba({},{},{},{:.3})", c.r, c.g, c.b, c.a as f32 / 255.0)
    }
}

fn write_html_paragraph(out: &mut String, p: &Paragraph) {
    let tag = match p.heading_level {
        Some(1) => "h1",
        Some(2) => "h2",
        _ => "p",
    };
    let has_tab = p.runs.iter().any(|r| r.text.contains('\t'));
    let is_left_tab = p
        .tab_stops
        .first()
        .is_some_and(|ts| ts.align == TabAlignment::Left);
    let has_dot_leader = p
        .tab_stops
        .first()
        .is_some_and(|ts| ts.leader == TabLeader::Dot);
    let mut classes = Vec::new();
    if p.list_kind == Some(ListKind::Bullet) {
        classes.push("list-bullet");
    }
    if has_tab && !is_left_tab {
        classes.push("tabbed-row");
        if has_dot_leader {
            classes.push("tabbed-dot");
        }
    }
    let class_attr = if classes.is_empty() {
        String::new()
    } else {
        format!(r#" class="{}""#, classes.join(" "))
    };

    let mut styles = Vec::new();
    let char_len: usize = p.runs.iter().map(|r| r.text.len()).sum();
    match p.alignment {
        Alignment::Center => styles.push("text-align:center".to_string()),
        Alignment::Right => {
            styles.push("text-align:right".to_string());
            if char_len <= 18 {
                styles.push("white-space:nowrap".to_string());
            }
        }
        Alignment::Justify => styles.push("text-align:justify".to_string()),
        Alignment::Left => {}
    }
    if p.space_before_pt > 0.2 {
        styles.push(format!("margin-top:{:.1}pt", p.space_before_pt));
    }
    if p.space_after_pt > 0.2 {
        styles.push(format!("margin-bottom:{:.1}pt", p.space_after_pt));
    }
    if p.left_indent_pt >= 3.0 {
        styles.push(format!("margin-left:{:.1}pt", p.left_indent_pt));
    }
    if let LineSpacingRule::ExactPt(pt) = p.line_spacing {
        styles.push(format!("line-height:{:.1}pt", pt));
    }

    let style_attr = if styles.is_empty() {
        String::new()
    } else {
        format!(r#" style="{}""#, styles.join(";"))
    };

    out.push_str(&format!("<{tag}{class_attr}{style_attr}>"));
    if has_tab {
        if is_left_tab {
            let tab_w = p
                .tab_stops
                .first()
                .map(|ts| (ts.pos_pt - p.left_indent_pt).clamp(48.0, 260.0))
                .unwrap_or(110.0);
            out.push_str(&format!(
                r#"<span style="display:inline-block;min-width:{tab_w:.1}pt;padding-right:8pt;">"#
            ));
        } else {
            out.push_str("<span>");
        }
    }
    for r in &p.runs {
        if r.text == "\t" {
            out.push_str("</span><span>");
            continue;
        }
        let mut span_styles = Vec::new();
        if r.style.font_size_pt > 1.0 {
            span_styles.push(format!("font-size:{:.1}pt", r.style.font_size_pt));
        }
        if r.style.width_scale_pct == 82 {
            span_styles.push(
                "font-family:'Oswald','Arial Narrow','Helvetica Neue',sans-serif".to_string(),
            );
        } else if r.style.font_family == "Nunito"
            || (r.style.font_family == "Avenir Next" && r.style.width_scale_pct == 99)
        {
            span_styles.push(
                "font-family:'Nunito','Avenir Next','Helvetica Neue',sans-serif".to_string(),
            );
        } else if r.style.font_family == "Avenir Next" {
            span_styles.push(
                "font-family:'Montserrat','Avenir Next','Helvetica Neue',sans-serif".to_string(),
            );
        } else if r.style.font_family == "Georgia" || r.style.font_family == "Times New Roman" {
            span_styles.push(format!(
                "font-family:'{}','Times New Roman',serif",
                xml_escape(&r.style.font_family)
            ));
        } else if !r.style.font_family.is_empty() {
            span_styles.push(format!(
                "font-family:'{}',sans-serif",
                xml_escape(&r.style.font_family)
            ));
        }
        if !r.style.color.is_near_black() {
            span_styles.push(format!("color:#{}", r.style.color.to_hex_rgb()));
        }
        let mut txt = xml_escape(&r.text);
        if r.style.bold {
            txt = format!("<strong>{txt}</strong>");
        }
        if r.style.italic {
            txt = format!("<em>{txt}</em>");
        }
        if !span_styles.is_empty() {
            txt = format!(r#"<span style="{}">{txt}</span>"#, span_styles.join(";"));
        }
        if let Some(uri) = &r.style.link_uri {
            txt = format!(r#"<a href="{}">{txt}</a>"#, xml_escape(uri));
        }
        out.push_str(&txt);
    }
    if has_tab {
        out.push_str("</span>");
    }
    out.push_str(&format!("</{tag}>\n"));
}

/// Serialize `FlowDoc` to GitHub-Flavored Markdown (GFM).
pub fn write_markdown(doc: &FlowDoc) -> String {
    let mut out = String::new();
    for sec in &doc.sections {
        for block in &sec.blocks {
            match block {
                FlowBlock::Paragraph(p) => {
                    let prefix = match p.heading_level {
                        Some(1) => "# ",
                        Some(2) => "## ",
                        _ => match p.list_kind {
                            Some(ListKind::Bullet) => "- ",
                            Some(ListKind::OrderedDecimal) => "1. ",
                            None => "",
                        },
                    };
                    let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                    if !text.trim().is_empty() {
                        out.push_str(prefix);
                        out.push_str(text.trim());
                        out.push_str("\n\n");
                    }
                }
                FlowBlock::Table(t) => {
                    for (r_idx, row) in t.rows.iter().enumerate() {
                        out.push('|');
                        for cell in &row.cells {
                            let cell_txt: String = cell
                                .paragraphs
                                .iter()
                                .flat_map(|p| p.runs.iter().map(|r| r.text.as_str()))
                                .collect::<Vec<_>>()
                                .join(" ");
                            out.push(' ');
                            out.push_str(cell_txt.trim());
                            out.push_str(" |");
                        }
                        out.push('\n');
                        if r_idx == 0 {
                            out.push('|');
                            for _ in &row.cells {
                                out.push_str(" --- |");
                            }
                            out.push('\n');
                        }
                    }
                    out.push('\n');
                }
                FlowBlock::TextBox(tb) => {
                    for p in &tb.paragraphs {
                        let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                        if !text.trim().is_empty() {
                            out.push_str(text.trim());
                            out.push_str("\n\n");
                        }
                    }
                }
                FlowBlock::Image(_) | FlowBlock::Shape(_) | FlowBlock::PageBreak => {}
            }
        }
    }
    out
}

/// Serialize `FlowDoc` to plain UTF-8 text (used by word-recall and glued-word QA metrics).
pub fn write_txt(doc: &FlowDoc) -> String {
    let mut out = String::new();
    for sec in &doc.sections {
        for p in &sec.header_paragraphs {
            let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            if !text.trim().is_empty() {
                out.push_str(text.trim());
                out.push('\n');
            }
        }
        for block in &sec.blocks {
            match block {
                FlowBlock::Paragraph(p) => {
                    let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                    if !text.trim().is_empty() {
                        out.push_str(text.trim());
                        out.push('\n');
                    }
                }
                FlowBlock::Table(t) => {
                    for row in &t.rows {
                        let row_txt: Vec<String> = row
                            .cells
                            .iter()
                            .map(|c| {
                                c.paragraphs
                                    .iter()
                                    .flat_map(|p| p.runs.iter().map(|r| r.text.as_str()))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .collect();
                        out.push_str(&row_txt.join("\t"));
                        out.push('\n');
                    }
                }
                FlowBlock::TextBox(tb) => {
                    for p in &tb.paragraphs {
                        let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                        if !text.trim().is_empty() {
                            out.push_str(text.trim());
                            out.push('\n');
                        }
                    }
                }
                FlowBlock::Image(_) | FlowBlock::Shape(_) | FlowBlock::PageBreak => {}
            }
        }
        for p in &sec.footer_paragraphs {
            let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            if !text.trim().is_empty() {
                out.push_str(text.trim());
                out.push('\n');
            }
        }
    }
    out
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((input.len() + 2) / 3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
