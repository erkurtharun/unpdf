//! A justified line's last space is advance, not ink — and with word spacing (`Tw`) it can be
//! wider than the gutter beside it.
//!
//! Every glyph of the font below is 500 units wide, so at 10 pt a character advances 5 pt and
//! `20 Tw` adds 20 pt to each space: `l01a l01b l01c ` inks 14 × 5 + 2 × 20 = 110 pt and then
//! advances 25 pt more. Two such columns 6 pt apart are two columns only if the left one ends
//! where its ink does.

mod common;

use common::{assemble, stream_object};
use unpdf::parser::backend::RawBackend;
use unpdf::parser::LayoutAnalyzer;

const LEFT_X: f32 = 40.0;
/// 6 pt to the right of where the left column's ink ends (40 + 110).
const RIGHT_X: f32 = 156.0;
const LINES: usize = 10;

fn monospaced_font() -> Vec<u8> {
    let widths = vec!["500"; 224].join(" ");
    format!(
        "<</Type/Font/Subtype/Type1/BaseFont/Mono/Encoding/WinAnsiEncoding\
         /FirstChar 32/LastChar 255/Widths[{widths}]>>"
    )
    .into_bytes()
}

fn pdf(content: &str) -> Vec<u8> {
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 400]/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content.as_bytes()),
        monospaced_font(),
    ])
}

/// Two justified columns, each line of the left one ending in a space.
fn two_columns() -> Vec<u8> {
    let mut content = String::from("BT /F1 10 Tf 20 Tw\n");
    for i in 0..LINES {
        let y = 360.0 - 12.0 * i as f32;
        let n = i + 1;
        content.push_str(&format!(
            "1 0 0 1 {LEFT_X} {y} Tm (l{n:02}a l{n:02}b l{n:02}c ) Tj\n\
             1 0 0 1 {RIGHT_X} {y} Tm (r{n:02}a r{n:02}b r{n:02}c) Tj\n"
        ));
    }
    content.push_str("ET");
    pdf(&content)
}

#[test]
fn a_justified_runs_ink_ends_before_its_trailing_space() {
    let doc = pdf("BT /F1 10 Tf 20 Tw 40 360 Td (l01a l01b l01c ) Tj ET");
    let backend = RawBackend::load_bytes(&doc).expect("fixture parses");
    let spans = LayoutAnalyzer::new(&backend)
        .extract_page_spans(1)
        .expect("spans");
    let span = spans
        .iter()
        .find(|s| s.text.starts_with("l01a"))
        .expect("the run");
    assert!(span.width_measured, "{span:?}");
    assert!(
        (span.width - 135.0).abs() < 0.5,
        "advance incl. the last space: {span:?}"
    );
    let ink = span.ink_width.expect("the trailing space is a word space");
    assert!((ink - 110.0).abs() < 0.5, "ink: {span:?}");
}

#[test]
fn justified_columns_with_a_gutter_narrower_than_the_last_space_are_two_columns() {
    let doc = unpdf::parse_bytes(&two_columns()).expect("parses");
    let text = doc.plain_text();
    let last_left = text
        .find(&format!("l{LINES:02}c"))
        .expect("left column text");
    let first_right = text.find("r01a").expect("right column text");
    assert!(
        last_left < first_right,
        "the left column is read before the right one:\n{text}"
    );
    for line in text.lines() {
        assert!(
            !(line.contains('l') && line.contains('r')),
            "a line joins the two columns across the gutter: {line:?}\n{text}"
        );
    }
}

/// `51 – 99 staff` broken after `51`: the next line opens with the en dash, and it is the same
/// sentence, not a list item — the line before it runs to the column's edge and closes
/// nothing. (WinAnsi 0x96 is the en dash.)
#[test]
fn a_dash_opening_a_wrapped_sentences_next_line_is_not_a_list_marker() {
    let content = "BT /F1 10 Tf\n\
        40 360 Td (those with six to fifty staff are small and those with 51) Tj\n\
        0 -12 Td (\\226 99 staff are medium.) Tj\n\
        0 -24 Td (Micro and small enterprises made up most of the sample.) Tj\n\
        ET";
    let doc = unpdf::parse_bytes(&pdf(content)).expect("parses");
    let md = unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default()).expect("renders");
    assert!(
        md.contains("those with 51 \u{2013} 99 staff are medium."),
        "the dash line continues the sentence, en dash kept:\n{md}"
    );
    assert!(!md.contains("\n- 99"), "no list item:\n{md}");
}
