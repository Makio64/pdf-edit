use std::collections::BTreeSet;
use p2d_ir::*;
use crate::zip_pkg::DeterministicZipWriter;

fn pt_to_twips(pt: f32) -> i32 {
    (pt * 20.0).round() as i32
}

fn pt_to_half_pt(pt: f32) -> i32 {
    (pt * 2.0).round().max(2.0) as i32
}

fn pt_to_emu(pt: f32) -> i64 {
    (pt as f64 * 12700.0).round() as i64
}

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\u{0000}'..='\u{0008}' | '\u{000B}' | '\u{000C}' | '\u{000E}'..='\u{001F}' => {}
            other => out.push(other),
        }
    }
    out
}

#[derive(Default)]
struct DocxRelBuilder {
    next_rid: usize,
    rels: Vec<(String, String, String, bool)>, // (rId, type, target, external)
    media_files: Vec<(String, String, Vec<u8>)>, // (zip_path, ext, bytes)
    header_files: Vec<(String, String)>, // (zip_path, xml)
    footer_files: Vec<(String, String)>, // (zip_path, xml)
    next_doc_pr_id: u32,
}

impl DocxRelBuilder {
    fn new() -> Self {
        let mut s = Self {
            next_rid: 1,
            rels: Vec::new(),
            media_files: Vec::new(),
            header_files: Vec::new(),
            footer_files: Vec::new(),
            next_doc_pr_id: 1,
        };
        s.add_internal_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
            "styles.xml",
        );
        s.add_internal_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings",
            "settings.xml",
        );
        s.add_internal_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable",
            "fontTable.xml",
        );
        s.add_internal_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering",
            "numbering.xml",
        );
        s
    }

    fn alloc_doc_pr_id(&mut self) -> u32 {
        let id = self.next_doc_pr_id;
        self.next_doc_pr_id += 1;
        id
    }

    fn add_internal_rel(&mut self, rel_type: &str, target: &str) -> String {
        let rid = format!("rId{}", self.next_rid);
        self.next_rid += 1;
        self.rels
            .push((rid.clone(), rel_type.to_string(), target.to_string(), false));
        rid
    }

    fn add_external_hyperlink(&mut self, uri: &str) -> String {
        let rid = format!("rId{}", self.next_rid);
        self.next_rid += 1;
        self.rels.push((
            rid.clone(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
                .to_string(),
            uri.to_string(),
            true,
        ));
        rid
    }

    fn add_image(&mut self, img: &AnchoredImage) -> String {
        let ext = if img.format == "png" { "png" } else { "jpeg" };
        let idx = self.media_files.len() + 1;
        let rel_target = format!("media/image{idx}.{ext}");
        let zip_path = format!("word/{rel_target}");
        self.media_files
            .push((zip_path, ext.to_string(), img.data.clone()));
        self.add_internal_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
            &rel_target,
        )
    }

    fn add_header(&mut self, xml: String) -> String {
        let idx = self.header_files.len() + 1;
        let rel_target = format!("header{idx}.xml");
        let zip_path = format!("word/{rel_target}");
        self.header_files.push((zip_path, xml));
        self.add_internal_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header",
            &rel_target,
        )
    }

    fn add_footer(&mut self, xml: String) -> String {
        let idx = self.footer_files.len() + 1;
        let rel_target = format!("footer{idx}.xml");
        let zip_path = format!("word/{rel_target}");
        self.footer_files.push((zip_path, xml));
        self.add_internal_rel(
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer",
            &rel_target,
        )
    }
}

/// Write a `FlowDoc` into a warning-free, standards-compliant `.docx` byte buffer.
pub fn write_docx(doc: &FlowDoc, params: &Params) -> Vec<u8> {
    let mut rels = DocxRelBuilder::new();
    let mut used_fonts = BTreeSet::new();
    let default_font = if doc.default_font_family.is_empty() {
        "Helvetica Neue"
    } else {
        doc.default_font_family.as_str()
    };
    used_fonts.insert(default_font.to_string());
    used_fonts.insert("Arial".to_string());
    used_fonts.insert("Times New Roman".to_string());
    used_fonts.insert("Georgia".to_string());
    used_fonts.insert("Menlo".to_string());

    let mut body_xml = String::with_capacity(16384);
    let sec_count = doc.sections.len().max(1);

    if doc.sections.is_empty() {
        body_xml.push_str("<w:p/>");
        body_xml.push_str(&write_sect_pr(&Section::default(), None, None, true));
    } else {
        for (sec_idx, sec) in doc.sections.iter().enumerate() {
            let is_last_sec = sec_idx + 1 == sec_count;

            let header_rid = if !sec.header_paragraphs.is_empty() {
                let mut h_xml = String::from(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
                );
                for p in &sec.header_paragraphs {
                    write_paragraph_xml(&mut h_xml, p, params, &mut rels, &mut used_fonts);
                }
                h_xml.push_str("</w:hdr>");
                Some(rels.add_header(h_xml))
            } else {
                None
            };

            let footer_rid = if !sec.footer_paragraphs.is_empty() {
                let mut f_xml = String::from(
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
                );
                for p in &sec.footer_paragraphs {
                    write_paragraph_xml(&mut f_xml, p, params, &mut rels, &mut used_fonts);
                }
                f_xml.push_str("</w:ftr>");
                Some(rels.add_footer(f_xml))
            } else {
                None
            };

            let mut bi = 0;
            while bi < sec.blocks.len() {
                // Page-anchored objects (background, positioned text boxes) share one empty anchor
                // paragraph, so dozens of them never push each other onto the next page.
                let anchored = sec.blocks[bi..]
                    .iter()
                    .take_while(|b| is_page_anchored(b))
                    .count();
                if anchored > 0 {
                    body_xml.push_str(ANCHOR_PARAGRAPH_START);
                    for block in &sec.blocks[bi..bi + anchored] {
                        match block {
                            FlowBlock::Image(img) => write_anchored_image_run_xml(&mut body_xml, img, &mut rels),
                            FlowBlock::TextBox(tb) => {
                                write_textbox_run_xml(&mut body_xml, tb, params, &mut rels, &mut used_fonts)
                            }
                            FlowBlock::Shape(shape) => write_shape_run_xml(&mut body_xml, shape, &mut rels),
                            _ => {}
                        }
                    }
                    body_xml.push_str("</w:p>");
                    bi += anchored;
                    continue;
                }
                let block = &sec.blocks[bi];
                bi += 1;
                match block {
                    FlowBlock::Paragraph(p) => {
                        let next_tbl_before = match sec.blocks.get(bi) {
                            Some(FlowBlock::Table(next_t)) => next_t.space_before_pt,
                            _ => 0.0,
                        };
                        if next_tbl_before > 0.5 {
                            let mut p_with_after = p.clone();
                            p_with_after.space_after_pt =
                                p_with_after.space_after_pt.max(next_tbl_before);
                            write_paragraph_xml(
                                &mut body_xml,
                                &p_with_after,
                                params,
                                &mut rels,
                                &mut used_fonts,
                            );
                        } else {
                            write_paragraph_xml(&mut body_xml, p, params, &mut rels, &mut used_fonts);
                        }
                    }
                    FlowBlock::Table(t) => {
                        let prev_was_para =
                            bi > 1 && matches!(sec.blocks.get(bi - 2), Some(FlowBlock::Paragraph(_)));
                        if !prev_was_para && t.space_before_pt >= 4.0 {
                            let gap_tw = pt_to_twips(t.space_before_pt);
                            body_xml.push_str(&format!(
                                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="{gap_tw}" w:line="20" w:lineRule="exact"/></w:pPr></w:p>"#
                            ));
                        }
                        write_table_xml(&mut body_xml, t, params, &mut rels, &mut used_fonts);
                    }
                    FlowBlock::Image(img) => {
                        write_image_paragraph_xml(&mut body_xml, img, sec, &mut rels);
                    }
                    FlowBlock::TextBox(tb) => {
                        write_textbox_paragraph_xml(
                            &mut body_xml,
                            tb,
                            params,
                            &mut rels,
                            &mut used_fonts,
                        );
                    }
                    FlowBlock::Shape(shape) => {
                        body_xml.push_str(ANCHOR_PARAGRAPH_START);
                        write_shape_run_xml(&mut body_xml, shape, &mut rels);
                        body_xml.push_str("</w:p>");
                    }
                    FlowBlock::PageBreak => {
                        body_xml.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
                    }
                }
            }

            let sect_pr = write_sect_pr(
                sec,
                header_rid.as_deref(),
                footer_rid.as_deref(),
                is_last_sec,
            );
            if is_last_sec {
                if sec.blocks.is_empty() {
                    body_xml.push_str("<w:p/>");
                }
                body_xml.push_str(&sect_pr);
            } else {
                // Multi-section break paragraph
                body_xml.push_str("<w:p><w:pPr>");
                body_xml.push_str(&sect_pr);
                body_xml.push_str("</w:pPr></w:p>");
            }
        }
    }

    let document_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
            xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
            xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
            xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"
            xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
  <w:body>{body_xml}</w:body>
</w:document>"#
    );

    let mut zip = DeterministicZipWriter::new();
    zip.add_file(
        "[Content_Types].xml",
        build_content_types_xml(&rels).as_bytes(),
        true,
    );
    zip.add_file("_rels/.rels", ROOT_RELS_XML.as_bytes(), true);
    zip.add_file(
        "word/_rels/document.xml.rels",
        build_document_rels_xml(&rels).as_bytes(),
        true,
    );
    zip.add_file(
        "word/styles.xml",
        build_styles_xml(default_font, doc.default_font_size_pt.max(11.0)).as_bytes(),
        true,
    );
    zip.add_file(
        "word/settings.xml",
        build_settings_xml(doc).as_bytes(),
        true,
    );
    zip.add_file(
        "word/fontTable.xml",
        build_font_table_xml(&used_fonts).as_bytes(),
        true,
    );
    zip.add_file("word/numbering.xml", NUMBERING_XML.as_bytes(), true);
    zip.add_file("word/document.xml", document_xml.as_bytes(), true);

    for (path, xml) in &rels.header_files {
        zip.add_file(path, xml.as_bytes(), true);
    }
    for (path, xml) in &rels.footer_files {
        zip.add_file(path, xml.as_bytes(), true);
    }
    for (path, _ext, bytes) in &rels.media_files {
        // Store already-compressed PNG/JPEG without re-deflating
        zip.add_file(path, bytes, false);
    }

    zip.finish()
}

fn write_sect_pr(
    sec: &Section,
    header_rid: Option<&str>,
    footer_rid: Option<&str>,
    _is_last: bool,
) -> String {
    let w = pt_to_twips(sec.page_width_pt);
    let h = pt_to_twips(sec.page_height_pt);
    let orient = if sec.page_width_pt > sec.page_height_pt {
        r#" w:orient="landscape""#
    } else {
        ""
    };
    let top = pt_to_twips(sec.margin_top_pt);
    let bot = pt_to_twips(sec.margin_bottom_pt);
    let left = pt_to_twips(sec.margin_left_pt);
    let right = pt_to_twips(sec.margin_right_pt);

    let mut out = String::from("<w:sectPr>");
    if let Some(hrid) = header_rid {
        out.push_str(&format!(r#"<w:headerReference w:type="default" r:id="{hrid}"/>"#));
    }
    if let Some(frid) = footer_rid {
        out.push_str(&format!(r#"<w:footerReference w:type="default" r:id="{frid}"/>"#));
    }
    out.push_str(&format!(r#"<w:pgSz w:w="{w}" w:h="{h}"{orient}/>"#));
    out.push_str(&format!(
        r#"<w:pgMar w:top="{top}" w:right="{right}" w:bottom="{bot}" w:left="{left}" w:header="450" w:footer="450" w:gutter="0"/>"#
    ));

    if sec.column_count > 1 {
        let space = pt_to_twips(sec.column_space_pt);
        if sec.column_widths_pt.len() == sec.column_count as usize {
            out.push_str(&format!(
                r#"<w:cols w:num="{}" w:space="{space}" w:equalWidth="0">"#,
                sec.column_count
            ));
            for (idx, &cw) in sec.column_widths_pt.iter().enumerate() {
                let cw_tw = pt_to_twips(cw);
                let sp = if idx + 1 < sec.column_widths_pt.len() {
                    space
                } else {
                    0
                };
                out.push_str(&format!(r#"<w:col w:w="{cw_tw}" w:space="{sp}"/>"#));
            }
            out.push_str("</w:cols>");
        } else {
            out.push_str(&format!(
                r#"<w:cols w:num="{}" w:space="{space}"/>"#,
                sec.column_count
            ));
        }
    }

    out.push_str("</w:sectPr>");
    out
}

fn write_paragraph_xml(
    out: &mut String,
    p: &Paragraph,
    params: &Params,
    rels: &mut DocxRelBuilder,
    used_fonts: &mut BTreeSet<String>,
) {
    out.push_str("<w:p><w:pPr>");

    if let Some(lvl) = p.heading_level {
        let style_id = match lvl {
            1 => "Heading1",
            _ => "Heading2",
        };
        out.push_str(&format!(r#"<w:pStyle w:val="{style_id}"/>"#));
    }

    if p.page_break_before {
        out.push_str("<w:pageBreakBefore/>");
    }
    if p.keep_with_next {
        out.push_str("<w:keepNext/>");
    }
    if !p.auto_hyphenation {
        out.push_str(r#"<w:suppressAutoHyphens w:val="1"/>"#);
    }

    if let Some(lk) = p.list_kind {
        let num_id = match lk {
            ListKind::Bullet => 1,
            ListKind::OrderedDecimal => 2,
        };
        out.push_str(&format!(
            r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="{num_id}"/></w:numPr>"#
        ));
    }

    if !p.tab_stops.is_empty() {
        out.push_str("<w:tabs>");
        for ts in &p.tab_stops {
            let val = match ts.align {
                TabAlignment::Left => "left",
                TabAlignment::Right => "right",
                TabAlignment::Center => "center",
                TabAlignment::Decimal => "decimal",
            };
            let leader = match ts.leader {
                TabLeader::None => "none",
                TabLeader::Dot => "dot",
                TabLeader::Underscore => "underscore",
            };
            let pos = pt_to_twips(ts.pos_pt).max(120);
            out.push_str(&format!(
                r#"<w:tab w:val="{val}" w:leader="{leader}" w:pos="{pos}"/>"#
            ));
        }
        out.push_str("</w:tabs>");
    }

    let before = pt_to_twips(p.space_before_pt).max(0);
    let after = pt_to_twips(p.space_after_pt).max(0);
    match p.line_spacing {
        LineSpacingRule::AtLeastPt(pt) => {
            let line = pt_to_twips(pt).max(200);
            out.push_str(&format!(
                r#"<w:spacing w:before="{before}" w:after="{after}" w:line="{line}" w:lineRule="atLeast"/>"#
            ));
        }
        LineSpacingRule::Multiple(mult) => {
            let line = (mult * 240.0).round().max(240.0) as i32;
            out.push_str(&format!(
                r#"<w:spacing w:before="{before}" w:after="{after}" w:line="{line}" w:lineRule="auto"/>"#
            ));
        }
        LineSpacingRule::ExactPt(pt) => {
            // Hard safety guard: never allow Exact line spacing in Editable preset!
            let rule = if params.preset == Preset::Editable {
                "atLeast"
            } else {
                "exact"
            };
            // Tiny print (legal notes at 5 pt) keeps its own pitch; only `atLeast` needs the 10 pt floor.
            let line = pt_to_twips(pt).max(if rule == "exact" { 40 } else { 200 });
            out.push_str(&format!(
                r#"<w:spacing w:before="{before}" w:after="{after}" w:line="{line}" w:lineRule="{rule}"/>"#
            ));
        }
    }

    if p.left_indent_pt > 0.5 || p.right_indent_pt > 0.5 || p.first_line_indent_pt.abs() > 0.5 {
        let left = pt_to_twips(p.left_indent_pt).max(0);
        let right = pt_to_twips(p.right_indent_pt).max(0);
        let first = pt_to_twips(p.first_line_indent_pt);
        if first >= 0 {
            out.push_str(&format!(
                r#"<w:ind w:left="{left}" w:right="{right}" w:firstLine="{first}"/>"#
            ));
        } else {
            let hanging = -first;
            out.push_str(&format!(
                r#"<w:ind w:left="{left}" w:right="{right}" w:hanging="{hanging}"/>"#
            ));
        }
    }

    let jc = match p.alignment {
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
        Alignment::Justify => "both",
    };
    out.push_str(&format!(r#"<w:jc w:val="{jc}"/>"#));
    out.push_str("</w:pPr>");

    for run in &p.runs {
        if let Some(uri) = &run.style.link_uri {
            let rid = rels.add_external_hyperlink(uri);
            out.push_str(&format!(r#"<w:hyperlink r:id="{rid}">"#));
            write_run_xml(out, run, used_fonts);
            out.push_str("</w:hyperlink>");
        } else {
            write_run_xml(out, run, used_fonts);
        }
    }

    out.push_str("</w:p>");
}

fn write_run_xml(out: &mut String, run: &TextRun, used_fonts: &mut BTreeSet<String>) {
    let family = if run.style.font_family.is_empty() {
        "Helvetica Neue"
    } else {
        run.style.font_family.as_str()
    };
    used_fonts.insert(family.to_string());
    let escaped_family = xml_escape(family);
    let sz = pt_to_half_pt(run.style.font_size_pt);

    out.push_str("<w:r><w:rPr>");
    // Critical Pages Zero-Warning fix: set all 4 slots (ascii, hAnsi, eastAsia, cs) to the Mac-installed font!
    out.push_str(&format!(
        r#"<w:rFonts w:ascii="{escaped_family}" w:hAnsi="{escaped_family}" w:eastAsia="{escaped_family}" w:cs="{escaped_family}"/>"#
    ));
    if run.style.bold {
        out.push_str("<w:b/><w:bCs/>");
    }
    if run.style.italic {
        out.push_str("<w:i/><w:iCs/>");
    }
    if run.style.underline || run.style.link_uri.is_some() {
        out.push_str(r#"<w:u w:val="single"/>"#);
    }
    if run.style.strikethrough {
        out.push_str("<w:strike/>");
    }
    if run.style.color != RgbaColor::BLACK {
        out.push_str(&format!(
            r#"<w:color w:val="{}"/>"#,
            run.style.color.to_hex_rgb()
        ));
    }
    let tracking = pt_to_twips(run.style.char_tracking_pt);
    if tracking != 0 {
        out.push_str(&format!(r#"<w:spacing w:val="{tracking}"/>"#));
    }
    if run.style.width_scale_pct != 100 && (50..=200).contains(&run.style.width_scale_pct) {
        out.push_str(&format!(r#"<w:w w:val="{}"/>"#, run.style.width_scale_pct));
    }
    if run.style.highlight_color.is_some() {
        out.push_str(r#"<w:highlight w:val="yellow"/>"#);
    }
    out.push_str(&format!(r#"<w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/>"#));
    out.push_str("</w:rPr>");

    // Split run text on `\t` and `\n` to emit proper `<w:tab/>` and `<w:br/>` elements
    let mut current_chunk = String::new();
    for ch in run.text.chars() {
        match ch {
            '\t' => {
                if !current_chunk.is_empty() {
                    out.push_str(&format!(
                        r#"<w:t xml:space="preserve">{}</w:t>"#,
                        xml_escape(&current_chunk)
                    ));
                    current_chunk.clear();
                }
                out.push_str("<w:tab/>");
            }
            '\n' => {
                if !current_chunk.is_empty() {
                    out.push_str(&format!(
                        r#"<w:t xml:space="preserve">{}</w:t>"#,
                        xml_escape(&current_chunk)
                    ));
                    current_chunk.clear();
                }
                out.push_str("<w:br/>");
            }
            other => current_chunk.push(other),
        }
    }
    if !current_chunk.is_empty() {
        out.push_str(&format!(
            r#"<w:t xml:space="preserve">{}</w:t>"#,
            xml_escape(&current_chunk)
        ));
    }

    out.push_str("</w:r>");
}

fn write_table_xml(
    out: &mut String,
    t: &TableGrid,
    params: &Params,
    rels: &mut DocxRelBuilder,
    used_fonts: &mut BTreeSet<String>,
) {
    let is_h_rule = t.is_borderless_layout
        && t.rows.len() == 1
        && t.col_widths_pt.len() == 1
        && t.rows[0].cells.first().is_some_and(|c| {
            c.border_bottom.is_some() && c.paragraphs.iter().all(|p| p.runs.is_empty())
        });
    if is_h_rule {
        if let Some(bdr) = t.rows[0].cells[0].border_bottom {
            let sz = (bdr.width_pt * 8.0).round().clamp(4.0, 24.0) as i32;
            let hex = bdr.color.to_hex_rgb();
            let before = pt_to_twips(t.space_before_pt.max(2.0));
            let after = pt_to_twips(t.space_after_pt.max(4.0));
            out.push_str(&format!(
                r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="{sz}" w:space="1" w:color="{hex}"/></w:pBdr><w:spacing w:before="{before}" w:after="{after}" w:line="60" w:lineRule="atLeast"/></w:pPr></w:p>"#
            ));
            return;
        }
    }

    let total_w: i32 = t
        .col_widths_pt
        .iter()
        .map(|&cw| pt_to_twips(cw).max(200))
        .sum::<i32>()
        .max(2000);
    let is_banner = t.is_borderless_layout
        && t.col_widths_pt.len() == 1
        && t.rows
            .first()
            .and_then(|r| r.cells.first())
            .is_some_and(|c| c.shading_color.is_some());

    out.push_str("<w:tbl><w:tblPr>");
    out.push_str(&format!(r#"<w:tblW w:w="{total_w}" w:type="dxa"/>"#));
    if t.left_indent_pt > 0.5 {
        let ind = pt_to_twips(t.left_indent_pt);
        out.push_str(&format!(r#"<w:tblInd w:w="{ind}" w:type="dxa"/>"#));
    }
    if t.is_borderless_layout {
        out.push_str(
            r#"<w:tblBorders>
                <w:top w:val="none" w:sz="0" w:space="0" w:color="auto"/>
                <w:left w:val="none" w:sz="0" w:space="0" w:color="auto"/>
                <w:bottom w:val="none" w:sz="0" w:space="0" w:color="auto"/>
                <w:right w:val="none" w:sz="0" w:space="0" w:color="auto"/>
                <w:insideH w:val="none" w:sz="0" w:space="0" w:color="auto"/>
                <w:insideV w:val="none" w:sz="0" w:space="0" w:color="auto"/>
            </w:tblBorders>"#,
        );
    } else {
        let bdr_hex = t
            .rows
            .first()
            .and_then(|r| r.cells.first())
            .and_then(|c| c.border_top)
            .map(|b| b.color.to_hex_rgb())
            .unwrap_or_else(|| "999999".to_string());
        out.push_str(&format!(
            r#"<w:tblBorders>
                <w:top w:val="single" w:sz="4" w:space="0" w:color="{bdr_hex}"/>
                <w:left w:val="single" w:sz="4" w:space="0" w:color="{bdr_hex}"/>
                <w:bottom w:val="single" w:sz="4" w:space="0" w:color="{bdr_hex}"/>
                <w:right w:val="single" w:sz="4" w:space="0" w:color="{bdr_hex}"/>
                <w:insideH w:val="single" w:sz="4" w:space="0" w:color="{bdr_hex}"/>
                <w:insideV w:val="single" w:sz="4" w:space="0" w:color="{bdr_hex}"/>
            </w:tblBorders>"#,
        ));
    }
    if is_banner {
        out.push_str(
            r#"<w:tblCellMar>
                <w:top w:w="140" w:type="dxa"/>
                <w:left w:w="180" w:type="dxa"/>
                <w:bottom w:w="140" w:type="dxa"/>
                <w:right w:w="180" w:type="dxa"/>
            </w:tblCellMar>"#,
        );
    } else if t.is_borderless_layout {
        out.push_str(
            r#"<w:tblCellMar>
                <w:top w:w="20" w:type="dxa"/>
                <w:left w:w="0" w:type="dxa"/>
                <w:bottom w:w="20" w:type="dxa"/>
                <w:right w:w="100" w:type="dxa"/>
            </w:tblCellMar>"#,
        );
    } else {
        out.push_str(
            r#"<w:tblCellMar>
                <w:top w:w="35" w:type="dxa"/>
                <w:left w:w="65" w:type="dxa"/>
                <w:bottom w:w="35" w:type="dxa"/>
                <w:right w:w="65" w:type="dxa"/>
            </w:tblCellMar>"#,
        );
    }
    out.push_str("</w:tblPr>");

    out.push_str("<w:tblGrid>");
    for &cw in &t.col_widths_pt {
        let w = pt_to_twips(cw).max(200);
        out.push_str(&format!(r#"<w:gridCol w:w="{w}"/>"#));
    }
    out.push_str("</w:tblGrid>");

    for row in &t.rows {
        out.push_str("<w:tr><w:trPr>");
        if row.is_header_row {
            out.push_str("<w:tblHeader/>");
        }
        if row.min_height_pt > 1.0 {
            let h = pt_to_twips(row.min_height_pt).max(160);
            out.push_str(&format!(r#"<w:trHeight w:val="{h}" w:hRule="atLeast"/>"#));
        }
        out.push_str("</w:trPr>");

        for cell in &row.cells {
            out.push_str("<w:tc><w:tcPr>");
            let cw = pt_to_twips(cell.width_pt).max(200);
            out.push_str(&format!(r#"<w:tcW w:w="{cw}" w:type="dxa"/>"#));
            if cell.grid_span > 1 {
                out.push_str(&format!(r#"<w:gridSpan w:val="{}"/>"#, cell.grid_span));
            }
            if let Some(restart) = cell.v_merge {
                if restart {
                    out.push_str(r#"<w:vMerge w:val="restart"/>"#);
                } else {
                    out.push_str("<w:vMerge/>");
                }
            }
            if t.is_borderless_layout {
                if let Some(bdr) = cell.border_top {
                    let hex = bdr.color.to_hex_rgb();
                    out.push_str(&format!(
                        r#"<w:tcBorders>
                            <w:top w:val="single" w:sz="4" w:space="0" w:color="{hex}"/>
                            <w:left w:val="single" w:sz="4" w:space="0" w:color="{hex}"/>
                            <w:bottom w:val="single" w:sz="4" w:space="0" w:color="{hex}"/>
                            <w:right w:val="single" w:sz="4" w:space="0" w:color="{hex}"/>
                        </w:tcBorders>"#
                    ));
                }
            } else if cell.border_top.is_none() || cell.border_bottom.is_none() {
                out.push_str("<w:tcBorders>");
                if cell.border_top.is_none() {
                    out.push_str(r#"<w:top w:val="none" w:sz="0" w:space="0" w:color="auto"/>"#);
                }
                if cell.border_bottom.is_none() {
                    out.push_str(r#"<w:bottom w:val="none" w:sz="0" w:space="0" w:color="auto"/>"#);
                }
                out.push_str("</w:tcBorders>");
            }
            if let Some(shd) = cell.shading_color {
                out.push_str(&format!(
                    r#"<w:shd w:val="clear" w:color="auto" w:fill="{}"/>"#,
                    shd.to_hex_rgb()
                ));
            }
            out.push_str("</w:tcPr>");

            if cell.paragraphs.is_empty() {
                out.push_str("<w:p/>");
            } else {
                for p in &cell.paragraphs {
                    write_paragraph_xml(out, p, params, rels, used_fonts);
                }
            }
            out.push_str("</w:tc>");
        }
        out.push_str("</w:tr>");
    }

    out.push_str("</w:tbl>");
}

fn write_image_paragraph_xml(
    out: &mut String,
    img: &AnchoredImage,
    sec: &Section,
    rels: &mut DocxRelBuilder,
) {
    if img.data.is_empty() {
        return;
    }
    if img.inline && !img.behind_doc {
        let rid = rels.add_image(img);
        let doc_pr_id = rels.alloc_doc_pr_id();
        let cx = pt_to_emu(img.bbox_pt.width().max(18.0));
        let cy = pt_to_emu(img.bbox_pt.height().max(18.0));
        let p_pr = if (img.bbox_pt.center_x() - sec.page_width_pt * 0.5).abs() <= 35.0 {
            r#"<w:pPr><w:jc w:val="center"/></w:pPr>"#.to_string()
        } else if img.bbox_pt.x0 > sec.margin_left_pt + 18.0 {
            let ind = pt_to_twips(
                (img.bbox_pt.x0 - sec.margin_left_pt).clamp(0.0, sec.page_width_pt * 0.55),
            );
            format!(r#"<w:pPr><w:ind w:left="{ind}"/></w:pPr>"#)
        } else {
            String::new()
        };
        out.push_str(&format!(
            r#"<w:p>{p_pr}<w:r><w:drawing>
                <wp:inline distT="0" distB="0" distL="0" distR="0">
                  <wp:extent cx="{cx}" cy="{cy}"/>
                  <wp:docPr id="{doc_pr_id}" name="Image {doc_pr_id}"/>
                  <a:graphic>
                    <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">
                      <pic:pic>
                        <pic:nvPicPr>
                          <pic:cNvPr id="{doc_pr_id}" name="Image {doc_pr_id}"/>
                          <pic:cNvPicPr/>
                        </pic:nvPicPr>
                        <pic:blipFill>
                          <a:blip r:embed="{rid}"/>
                          <a:stretch><a:fillRect/></a:stretch>
                        </pic:blipFill>
                        <pic:spPr>
                          <a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>
                          <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                        </pic:spPr>
                      </pic:pic>
                    </a:graphicData>
                  </a:graphic>
                </wp:inline>
            </w:drawing></w:r></w:p>"#
        ));
    } else {
        out.push_str("<w:p>");
        write_anchored_image_run_xml(out, img, rels);
        out.push_str("</w:p>");
    }
}

/// Paragraph that only carries page-anchored drawings: no height, so it never moves content.
const ANCHOR_PARAGRAPH_START: &str = r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="20" w:lineRule="exact"/><w:rPr><w:sz w:val="2"/><w:szCs w:val="2"/></w:rPr></w:pPr>"#;

/// Blocks positioned relative to the page rather than flowing with the text.
fn is_page_anchored(block: &FlowBlock) -> bool {
    match block {
        FlowBlock::TextBox(_) | FlowBlock::Shape(_) => true,
        FlowBlock::Image(img) => !img.data.is_empty() && (!img.inline || img.behind_doc),
        _ => false,
    }
}

/// `<a:srgbClr>` for `c`, with its transparency.
fn drawing_color_xml(c: RgbaColor) -> String {
    if c.a < 255 {
        let alpha = (c.a as u32 * 100_000) / 255;
        format!(r#"<a:srgbClr val="{}"><a:alpha val="{alpha}"/></a:srgbClr>"#, c.to_hex_rgb())
    } else {
        format!(r#"<a:srgbClr val="{}"/>"#, c.to_hex_rgb())
    }
}

/// A run holding `shape` as a rectangle anchored at its page position, behind the text.
fn write_shape_run_xml(out: &mut String, shape: &ShapeBlock, rels: &mut DocxRelBuilder) {
    let doc_pr_id = rels.alloc_doc_pr_id();
    let b = shape.bbox_pt;
    let (pos_x, pos_y) = (pt_to_emu(b.x0), pt_to_emu(b.y0));
    let (cx, cy) = (pt_to_emu(b.width()).max(1), pt_to_emu(b.height()).max(1));
    let fill = match shape.fill_color {
        Some(c) => format!("<a:solidFill>{}</a:solidFill>", drawing_color_xml(c)),
        None => "<a:noFill/>".to_string(),
    };
    let line = match shape.stroke {
        Some(s) => format!(
            r#"<a:ln w="{}"><a:solidFill>{}</a:solidFill><a:miter lim="800000"/></a:ln>"#,
            pt_to_emu(s.width_pt).max(1),
            drawing_color_xml(s.color)
        ),
        None => "<a:ln><a:noFill/></a:ln>".to_string(),
    };
    // Drawing order is kept through increasing z-order, all behind the text.
    let z = 2 + doc_pr_id;
    out.push_str(&format!(
        r#"<w:r><w:rPr><w:sz w:val="2"/><w:szCs w:val="2"/></w:rPr><w:drawing>
            <wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="{z}" behindDoc="1" locked="0" layoutInCell="1" allowOverlap="1">
              <wp:simplePos x="0" y="0"/>
              <wp:positionH relativeFrom="page"><wp:posOffset>{pos_x}</wp:posOffset></wp:positionH>
              <wp:positionV relativeFrom="page"><wp:posOffset>{pos_y}</wp:posOffset></wp:positionV>
              <wp:extent cx="{cx}" cy="{cy}"/>
              <wp:wrapNone/>
              <wp:docPr id="{doc_pr_id}" name="Shape {doc_pr_id}"/>
              <a:graphic>
                <a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                  <wps:wsp>
                    <wps:cNvSpPr/>
                    <wps:spPr>
                      <a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>
                      <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                      {fill}{line}
                    </wps:spPr>
                    <wps:bodyPr/>
                  </wps:wsp>
                </a:graphicData>
              </a:graphic>
            </wp:anchor>
        </w:drawing></w:r>"#
    ));
}

/// A run holding `img` anchored at its page position (behind the text when `behind_doc`).
fn write_anchored_image_run_xml(out: &mut String, img: &AnchoredImage, rels: &mut DocxRelBuilder) {
    if img.data.is_empty() {
        return;
    }
    let rid = rels.add_image(img);
    let doc_pr_id = rels.alloc_doc_pr_id();
    let cx = pt_to_emu(img.bbox_pt.width().max(18.0));
    let cy = pt_to_emu(img.bbox_pt.height().max(18.0));
    let pos_x = pt_to_emu(img.bbox_pt.x0.max(0.0));
    let pos_y = pt_to_emu(img.bbox_pt.y0.max(0.0));
    let behind = if img.behind_doc { "1" } else { "0" };
    out.push_str(&format!(
        r#"<w:r><w:rPr><w:sz w:val="2"/><w:szCs w:val="2"/></w:rPr><w:drawing>
                <wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="{behind}" locked="0" layoutInCell="1" allowOverlap="1">
                  <wp:simplePos x="0" y="0"/>
                  <wp:positionH relativeFrom="page"><wp:posOffset>{pos_x}</wp:posOffset></wp:positionH>
                  <wp:positionV relativeFrom="page"><wp:posOffset>{pos_y}</wp:posOffset></wp:positionV>
                  <wp:extent cx="{cx}" cy="{cy}"/>
                  <wp:wrapNone/>
                  <wp:docPr id="{doc_pr_id}" name="Image {doc_pr_id}"/>
                  <a:graphic>
                    <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">
                      <pic:pic>
                        <pic:nvPicPr>
                          <pic:cNvPr id="{doc_pr_id}" name="Image {doc_pr_id}"/>
                          <pic:cNvPicPr/>
                        </pic:nvPicPr>
                        <pic:blipFill>
                          <a:blip r:embed="{rid}"/>
                          <a:stretch><a:fillRect/></a:stretch>
                        </pic:blipFill>
                        <pic:spPr>
                          <a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>
                          <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                        </pic:spPr>
                      </pic:pic>
                    </a:graphicData>
                  </a:graphic>
                </wp:anchor>
            </w:drawing></w:r>"#
    ));
}

fn write_textbox_paragraph_xml(
    out: &mut String,
    tb: &TextBoxBlock,
    params: &Params,
    rels: &mut DocxRelBuilder,
    used_fonts: &mut BTreeSet<String>,
) {
    out.push_str(ANCHOR_PARAGRAPH_START);
    write_textbox_run_xml(out, tb, params, rels, used_fonts);
    out.push_str("</w:p>");
}

/// A run holding `tb` as a borderless text box anchored at its page position, with zero insets so
/// its first baseline lands `0.8 × line height` below the box top (see `p2d_layout::exact`).
fn write_textbox_run_xml(
    out: &mut String,
    tb: &TextBoxBlock,
    params: &Params,
    rels: &mut DocxRelBuilder,
    used_fonts: &mut BTreeSet<String>,
) {
    let doc_pr_id = rels.alloc_doc_pr_id();
    let pos_x = pt_to_emu(tb.bbox_pt.x0);
    let pos_y = pt_to_emu(tb.bbox_pt.y0);
    // Add 15% height slack in EXACT text boxes so substituted fonts never clip
    let cx = pt_to_emu(tb.bbox_pt.width().max(8.0));
    let cy = pt_to_emu((tb.bbox_pt.height() * 1.15).max(8.0));

    let mut inner_p = String::new();
    for p in &tb.paragraphs {
        write_paragraph_xml(&mut inner_p, p, params, rels, used_fonts);
    }
    if inner_p.is_empty() {
        inner_p.push_str("<w:p/>");
    }

    let autofit_xml = if tb.auto_fit_height {
        "<a:spAutoFit/>"
    } else {
        "<a:noAutofit/>"
    };

    out.push_str(&format!(
        r#"<w:r><w:rPr><w:sz w:val="2"/><w:szCs w:val="2"/></w:rPr><w:drawing>
            <wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="10" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">
              <wp:simplePos x="0" y="0"/>
              <wp:positionH relativeFrom="page"><wp:posOffset>{pos_x}</wp:posOffset></wp:positionH>
              <wp:positionV relativeFrom="page"><wp:posOffset>{pos_y}</wp:posOffset></wp:positionV>
              <wp:extent cx="{cx}" cy="{cy}"/>
              <wp:wrapNone/>
              <wp:docPr id="{doc_pr_id}" name="TextBox {doc_pr_id}"/>
              <a:graphic>
                <a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
                  <wps:wsp>
                    <wps:cNvSpPr txBox="1"/>
                    <wps:spPr>
                      <a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>
                      <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
                      <a:noFill/>
                      <a:ln><a:noFill/></a:ln>
                    </wps:spPr>
                    <wps:txbx><w:txbxContent>{inner_p}</w:txbxContent></wps:txbx>
                    <wps:bodyPr wrap="square" lIns="0" tIns="0" rIns="0" bIns="0" anchor="t">{autofit_xml}</wps:bodyPr>
                  </wps:wsp>
                </a:graphicData>
              </a:graphic>
            </wp:anchor>
        </w:drawing></w:r>"#
    ));
}

const ROOT_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

fn build_content_types_xml(rels: &DocxRelBuilder) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Default Extension="jpeg" ContentType="image/jpeg"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
  <Override PartName="/word/fontTable.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"/>
  <Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>"#,
    );
    for (zip_path, _) in &rels.header_files {
        xml.push_str(&format!(
            r#"<Override PartName="/{zip_path}" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#
        ));
    }
    for (zip_path, _) in &rels.footer_files {
        xml.push_str(&format!(
            r#"<Override PartName="/{zip_path}" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#
        ));
    }
    xml.push_str("</Types>");
    xml
}

fn build_document_rels_xml(rels: &DocxRelBuilder) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (rid, rtype, target, external) in &rels.rels {
        let mode = if *external {
            r#" TargetMode="External""#
        } else {
            ""
        };
        xml.push_str(&format!(
            r#"<Relationship Id="{rid}" Type="{rtype}" Target="{}"{mode}/>"#,
            xml_escape(target)
        ));
    }
    xml.push_str("</Relationships>");
    xml
}

fn build_styles_xml(default_font: &str, default_size_pt: f32) -> String {
    let f = xml_escape(default_font);
    let sz = pt_to_half_pt(default_size_pt);
    let h1_sz = pt_to_half_pt((default_size_pt * 1.6).max(16.0));
    let h2_sz = pt_to_half_pt((default_size_pt * 1.3).max(13.5));

    // Zero-Warning Pages Profile: all 4 rFonts slots explicitly set to Mac-installed `default_font`!
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault>
      <w:rPr>
        <w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:eastAsia="{f}" w:cs="{f}"/>
        <w:sz w:val="{sz}"/>
        <w:szCs w:val="{sz}"/>
        <w:lang w:val="fr-FR" w:eastAsia="fr-FR" w:bidi="ar-SA"/>
      </w:rPr>
    </w:rPrDefault>
    <w:pPrDefault>
      <w:pPr>
        <w:spacing w:after="0" w:line="240" w:lineRule="auto"/>
      </w:pPr>
    </w:pPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:name w:val="Normal"/>
    <w:qFormat/>
    <w:rPr>
      <w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:eastAsia="{f}" w:cs="{f}"/>
      <w:sz w:val="{sz}"/>
      <w:szCs w:val="{sz}"/>
    </w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Heading1">
    <w:name w:val="heading 1"/>
    <w:basedOn w:val="Normal"/>
    <w:next w:val="Normal"/>
    <w:qFormat/>
    <w:pPr>
      <w:keepNext/>
      <w:spacing w:before="200" w:after="100"/>
      <w:outlineLvl w:val="0"/>
    </w:pPr>
    <w:rPr>
      <w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:eastAsia="{f}" w:cs="{f}"/>
      <w:b/>
      <w:bCs/>
      <w:sz w:val="{h1_sz}"/>
      <w:szCs w:val="{h1_sz}"/>
    </w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="Heading2">
    <w:name w:val="heading 2"/>
    <w:basedOn w:val="Normal"/>
    <w:next w:val="Normal"/>
    <w:qFormat/>
    <w:pPr>
      <w:keepNext/>
      <w:spacing w:before="160" w:after="80"/>
      <w:outlineLvl w:val="1"/>
    </w:pPr>
    <w:rPr>
      <w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:eastAsia="{f}" w:cs="{f}"/>
      <w:b/>
      <w:bCs/>
      <w:sz w:val="{h2_sz}"/>
      <w:szCs w:val="{h2_sz}"/>
    </w:rPr>
  </w:style>
</w:styles>"#
    )
}

fn build_settings_xml(doc: &FlowDoc) -> String {
    let any_hyphen = doc
        .sections
        .iter()
        .flat_map(|s| s.blocks.iter())
        .any(|b| matches!(b, FlowBlock::Paragraph(p) if p.auto_hyphenation));
    let hyph_tag = if any_hyphen {
        r#"<w:autoHyphenation w:val="1"/>"#
    } else {
        r#"<w:autoHyphenation w:val="0"/>"#
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:zoom w:percent="100"/>
  {hyph_tag}
  <w:defaultTabStop w:val="708"/>
  <w:compat>
    <w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>
  </w:compat>
</w:settings>"#
    )
}

fn build_font_table_xml(used_fonts: &BTreeSet<String>) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:fonts xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
    );
    for font in used_fonts {
        let escaped = xml_escape(font);
        let family = if font.contains("Times") || font.contains("Georgia") {
            "roman"
        } else if font.contains("Menlo") || font.contains("Courier") {
            "modern"
        } else {
            "swiss"
        };
        xml.push_str(&format!(
            r#"<w:font w:name="{escaped}">
                <w:charset w:val="00"/>
                <w:family w:val="{family}"/>
                <w:pitch w:val="variable"/>
              </w:font>"#
        ));
    }
    xml.push_str("</w:fonts>");
    xml
}

const NUMBERING_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="0">
    <w:multiLevelType w:val="hybridMultilevel"/>
    <w:lvl w:ilvl="0">
      <w:start w:val="1"/>
      <w:numFmt w:val="bullet"/>
      <w:lvlText w:val="•"/>
      <w:lvlJc w:val="left"/>
      <w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr>
      <w:rPr><w:rFonts w:ascii="Helvetica Neue" w:hAnsi="Helvetica Neue" w:eastAsia="Helvetica Neue" w:cs="Helvetica Neue"/></w:rPr>
    </w:lvl>
  </w:abstractNum>
  <w:abstractNum w:abstractNumId="1">
    <w:multiLevelType w:val="hybridMultilevel"/>
    <w:lvl w:ilvl="0">
      <w:start w:val="1"/>
      <w:numFmt w:val="decimal"/>
      <w:lvlText w:val="%1."/>
      <w:lvlJc w:val="left"/>
      <w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr>
      <w:rPr><w:rFonts w:ascii="Helvetica Neue" w:hAnsi="Helvetica Neue" w:eastAsia="Helvetica Neue" w:cs="Helvetica Neue"/></w:rPr>
    </w:lvl>
  </w:abstractNum>
  <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
  <w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
</w:numbering>"#;
