use p2d_ir::*;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};

/// Write a searchable PDF from `FlowDoc` using `pdf-writer`.
pub fn write_searchable_pdf(doc: &FlowDoc) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_id = Ref::new(3);
    let mut next_id = 4;

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut page_ids = Vec::new();
    let sections = if doc.sections.is_empty() {
        vec![Section::default()]
    } else {
        doc.sections.clone()
    };

    for sec in &sections {
        let page_id = Ref::new(next_id);
        let content_id = Ref::new(next_id + 1);
        next_id += 2;
        page_ids.push(page_id);

        let mut content = Content::new();
        let page_h = sec.page_height_pt;

        for block in &sec.blocks {
            if let FlowBlock::Paragraph(p) = block {
                let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                let win_bytes = encode_win_ansi(&text);
                let x = p.source_bbox.x0.max(sec.margin_left_pt);
                let y = (page_h - p.source_bbox.y1).clamp(24.0, page_h - 24.0);
                let sz = p
                    .runs
                    .first()
                    .map(|r| r.style.font_size_pt)
                    .unwrap_or(11.0);
                content.begin_text();
                content.set_font(Name(b"F1"), sz);
                content.next_line(x, y);
                content.show(Str(&win_bytes));
                content.end_text();
            }
        }

        pdf.stream(content_id, &content.finish());

        let mut page = pdf.page(page_id);
        page.parent(page_tree_id);
        page.media_box(Rect::new(0.0, 0.0, sec.page_width_pt, sec.page_height_pt));
        page.resources().fonts().pair(Name(b"F1"), font_id);
        page.contents(content_id);
        page.finish();
    }

    pdf.pages(page_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);

    pdf.finish()
}

pub fn encode_win_ansi(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for ch in s.chars() {
        let b = match ch {
            '\u{0020}'..='\u{007E}' | '\u{00A0}'..='\u{00FF}' => ch as u8,
            '€' => 0x80,
            '’' => 0x92,
            '‘' => 0x91,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            'œ' => 0x9C,
            'Œ' => 0x8C,
            '\t' => b' ',
            _ => b'?',
        };
        out.push(b);
    }
    out
}
