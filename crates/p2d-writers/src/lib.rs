pub mod docx;
pub mod pages;
pub mod spdf;
pub mod web;
pub mod zip_pkg;

use p2d_ir::*;

/// Serialize a `FlowDoc` into the requested `OutputFormat`.
pub fn serialize_document(doc: &FlowDoc, params: &Params) -> Vec<u8> {
    match params.format {
        OutputFormat::Docx => docx::write_docx(doc, params),
        // Empty without Pages 14.5's template: the app and the browser build ask Pages for a DOCX
        // instead, and the CLI checks `pages::blank_template()` first.
        OutputFormat::Pages => pages::write_pages(doc).unwrap_or_default(),
        OutputFormat::Html => web::write_html(doc).into_bytes(),
        OutputFormat::Markdown => web::write_markdown(doc).into_bytes(),
        OutputFormat::Txt => web::write_txt(doc).into_bytes(),
        OutputFormat::SearchablePdf => spdf::write_searchable_pdf(doc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_warning_pages_docx_invariants() {
        let mut sec = Section::default();
        sec.blocks.push(FlowBlock::Paragraph(Paragraph {
            runs: vec![TextRun {
                text: "Facture de test — 120,00\u{00A0}€".to_string(),
                style: RunStyle::default(),
            }],
            line_spacing: LineSpacingRule::AtLeastPt(13.5),
            ..Default::default()
        }));
        let doc = FlowDoc {
            sections: vec![sec],
            ..Default::default()
        };
        let bytes = serialize_document(&doc, &Params::default());
        assert!(bytes.starts_with(b"PK\x03\x04"));
        let raw_ascii = String::from_utf8_lossy(&bytes);
        // Must NOT contain Office theme1.xml
        assert!(!raw_ascii.contains("theme1.xml"));
        // Must contain document.xml and styles.xml
        assert!(raw_ascii.contains("word/document.xml"));
        assert!(raw_ascii.contains("word/styles.xml"));
    }
}
