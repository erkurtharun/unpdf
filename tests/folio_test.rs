//! A page number is left out by where it sits — the page's own margin, below (or above) all
//! other text — not by being a number: a chart's value or a table's row keeps its numbers.

mod common;

use common::page_pdf;

const LETTER: &str = "/MediaBox[0 0 612 792]";

fn markdown(content: &str) -> String {
    let doc = unpdf::parse_bytes(&page_pdf("", LETTER, content.as_bytes())).expect("parses");
    unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default()).expect("renders")
}

const BODY: &str = "BT /F1 11 Tf 72 700 Td (A paragraph of body text on the page.) Tj ET\n\
                    BT /F1 11 Tf 72 686 Td (It goes on for a second line.) Tj ET\n";

/// A folio 9% above the bottom edge — inside the page's margin, outside the 5% band.
#[test]
fn a_folio_in_the_bottom_margin_is_left_out() {
    let md = markdown(&format!("{BODY}BT /F1 11 Tf 500 72 Td (362) Tj ET"));
    assert!(md.contains("body text"), "{md}");
    assert!(!md.contains("362"), "{md}");
}

/// A running title at one side and the folio at the other share a baseline.
#[test]
fn a_folio_beside_a_running_title_is_left_out() {
    let md = markdown(&format!(
        "{BODY}BT /F1 8 Tf 72 72 Td (Milken Institute School of Public Health) Tj ET\n\
         BT /F1 8 Tf 560 72 Td (6) Tj ET"
    ));
    assert!(!md.contains(" 6") && !md.contains("\n6"), "{md}");
}

/// A chart's values sit mid-page, each a line of its own: content, not page numbers.
#[test]
fn a_charts_values_mid_page_are_kept() {
    let md = markdown(&format!(
        "{BODY}BT /F1 7 Tf 100 500 Td (100) Tj ET\nBT /F1 7 Tf 100 450 Td (80) Tj ET\n\
         BT /F1 7 Tf 100 400 Td (60) Tj ET"
    ));
    for value in ["100", "80", "60"] {
        assert!(md.contains(value), "{value} kept:\n{md}");
    }
}

/// The last row of a table near the bottom has more than one number on its baseline.
#[test]
fn a_tables_last_row_near_the_bottom_is_kept() {
    let md = markdown(&format!(
        "{BODY}BT /F1 10 Tf 72 80 Td (12) Tj ET\nBT /F1 10 Tf 200 80 Td (15) Tj ET\n\
         BT /F1 10 Tf 330 80 Td (27) Tj ET"
    ));
    for value in ["12", "15", "27"] {
        assert!(md.contains(value), "{value} kept:\n{md}");
    }
}
