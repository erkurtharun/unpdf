//! Plain text rendering for PDF documents.

use crate::error::Result;
use crate::model::{Block, Document};

use super::{CleanupPipeline, RenderOptions};

/// Convert a document to plain text.
///
/// Renders the pages `options.page_selection` selects; a selection naming a page the
/// document does not have fails as [`to_markdown`](super::to_markdown) does — see
/// [`PageSelection::check`](super::PageSelection::check).
pub fn to_text(doc: &Document, options: &RenderOptions) -> Result<String> {
    options.page_selection.check_document(doc)?;
    let pages = doc
        .pages
        .iter()
        .filter(|page| options.page_selection.includes(page.number));
    let Some(ref cleanup_options) = options.cleanup else {
        let output = pages.map(|page| page.plain_text()).collect::<Vec<_>>();
        return Ok(output.join("\n\n").trim().to_string());
    };

    // Clean-up joins and rewrites prose; a code block's lines are literal. So the text is
    // cleaned a run of prose blocks at a time, and each code block is passed through as is.
    let pipeline = CleanupPipeline::new(cleanup_options.clone());
    let mut parts: Vec<String> = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let flush = |prose: &mut Vec<String>, parts: &mut Vec<String>| {
        if !prose.is_empty() {
            let cleaned = pipeline.process(&prose.join("\n\n"));
            prose.clear();
            if !cleaned.trim().is_empty() {
                parts.push(cleaned.trim().to_string());
            }
        }
    };
    for block in pages.flat_map(|page| page.elements.iter()) {
        let mut text = String::new();
        block.append_plain_text(&mut text);
        if text.is_empty() {
            continue;
        }
        if matches!(block, Block::Code { .. }) {
            flush(&mut prose, &mut parts);
            parts.push(text);
        } else {
            prose.push(text);
        }
    }
    flush(&mut prose, &mut parts);
    Ok(parts.join("\n\n").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Page, Paragraph};

    #[test]
    fn test_to_text() {
        let mut doc = Document::new();
        let mut page = Page::letter(1);
        page.add_paragraph(Paragraph::with_text("Hello, world!"));
        page.add_paragraph(Paragraph::with_text("Second paragraph."));
        doc.add_page(page);

        let options = RenderOptions::default();
        let result = to_text(&doc, &options).unwrap();

        assert!(result.contains("Hello, world!"));
        assert!(result.contains("Second paragraph."));
    }

    fn two_page_doc() -> Document {
        let mut doc = Document::new();
        for (number, text) in [(1, "Page one"), (2, "Page two")] {
            let mut page = Page::letter(number);
            page.add_paragraph(Paragraph::with_text(text));
            doc.add_page(page);
        }
        doc
    }

    #[test]
    fn to_text_renders_the_selected_pages() {
        let options = RenderOptions::default().with_page_list(vec![2]);
        assert_eq!(to_text(&two_page_doc(), &options).unwrap(), "Page two");
    }

    #[test]
    fn to_text_rejects_a_selection_naming_a_missing_page() {
        let options = RenderOptions::default().with_page_list(vec![5]);
        assert!(matches!(
            to_text(&two_page_doc(), &options),
            Err(crate::Error::PageOutOfRange(5, 2))
        ));
    }
}
