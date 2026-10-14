//! End-to-end lattice-mode table detection: real PDF bytes with drawn ruling
//! lines, through the actual public parse API — not unit-level `ContentOp`
//! construction. Confirms the extraction → grid-inference → cell-assignment
//! pipeline is actually wired into `PdfParser`'s output, not just internally
//! self-consistent.

mod common;

use unpdf::model::Block;
use unpdf::PdfParser;

#[test]
fn bordered_grid_is_extracted_as_a_table() {
    let doc = PdfParser::from_bytes(&common::bordered_table_pdf())
        .and_then(|p| p.parse())
        .expect("synthetic bordered-table PDF should parse");

    assert_eq!(doc.pages.len(), 1);
    let tables: Vec<&unpdf::model::Table> = doc.pages[0]
        .elements
        .iter()
        .filter_map(|b| match b {
            Block::Table(t) => Some(t),
            _ => None,
        })
        .collect();

    assert_eq!(
        tables.len(),
        1,
        "expected exactly one table from the drawn 2x2 grid, got elements: {:?}",
        doc.pages[0].elements
    );

    let table = tables[0];
    assert_eq!(table.row_count(), 2);
    assert_eq!(table.column_count(), 2);
    assert_eq!(table.rows[0].cells[0].plain_text(), "Name");
    assert_eq!(table.rows[0].cells[1].plain_text(), "Age");
    assert_eq!(table.rows[1].cells[0].plain_text(), "Alice");
    assert_eq!(table.rows[1].cells[1].plain_text(), "30");
}

#[test]
fn text_between_unrelated_ruling_lines_is_not_a_table() {
    let doc = PdfParser::from_bytes(&common::figure_frame_and_rule_pdf())
        .and_then(|p| p.parse())
        .expect("synthetic figure-and-rule PDF should parse");

    let elements = &doc.pages[0].elements;
    assert!(
        !elements.iter().any(|b| matches!(b, Block::Table(_))),
        "a figure frame and a distant rule are not a table: {elements:?}"
    );
    let text = doc.plain_text();
    assert!(
        text.contains("Development of the concept"),
        "the heading must come through whole, got: {text:?}"
    );
}

#[test]
fn body_lines_drawn_in_aligned_runs_are_not_a_table() {
    // Run boundaries that happen to line up from row to row are not column edges when
    // the runs abut: a column starts after a gap, not in the middle of a word.
    let doc = PdfParser::from_bytes(&common::fragmented_body_lines_pdf())
        .and_then(|p| p.parse())
        .expect("synthetic fragmented-lines PDF should parse");
    let elements = &doc.pages[0].elements;
    assert!(
        !elements.iter().any(|b| matches!(b, Block::Table(_))),
        "abutting runs must not be split into table columns: {elements:?}"
    );
}

/// A table of two columns ruled only between them -- one column rule, rules between the rows
/// running past it to the table's sides, a shaded header band as wide as the rules -- with a
/// row label beside a cell of two lines, and a bar drawn with a wide pen in a cell beside its
/// value. One table: a row per ruled row, the cell's lines read as one, the bar no rule.
#[test]
fn two_columns_ruled_only_between_them_are_one_table() {
    let content = b"0.9 g 60 680 340 20 re f 0 g 0.5 w \
        60 700 m 400 700 l S \
        60 680 m 400 680 l S \
        60 640 m 400 640 l S \
        60 600 m 400 600 l S \
        130 600 m 130 700 l S \
        5 w 130 620 m 250 620 l S 0.5 w \
        BT /F1 10 Tf 70 686 Td (Term) Tj ET \
        BT /F1 10 Tf 140 686 Td (Provisions) Tj ET \
        BT /F1 10 Tf 70 656 Td (Ends) Tj ET \
        BT /F1 10 Tf 140 666 Td (First item) Tj ET \
        BT /F1 10 Tf 140 650 Td (Second item) Tj ET \
        BT /F1 10 Tf 70 616 Td (Extended) Tj ET \
        BT /F1 10 Tf 255 616 Td (12 years) Tj ET\n";
    let pdf = common::page_pdf("", "/MediaBox[0 0 595 842]", content);
    let doc = PdfParser::from_bytes(&pdf)
        .and_then(|p| p.parse())
        .expect("synthetic open two-column table PDF should parse");
    let tables: Vec<&unpdf::model::Table> = doc.pages[0]
        .elements
        .iter()
        .filter_map(|b| match b {
            Block::Table(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 1, "{:?}", doc.pages[0].elements);
    let rows: Vec<Vec<String>> = tables[0]
        .rows
        .iter()
        .map(|r| r.cells.iter().map(|c| c.plain_text()).collect())
        .collect();
    assert_eq!(
        rows,
        vec![
            vec!["Term".to_string(), "Provisions".to_string()],
            vec!["Ends".to_string(), "First item Second item".to_string()],
            vec!["Extended".to_string(), "12 years".to_string()],
        ]
    );
}
