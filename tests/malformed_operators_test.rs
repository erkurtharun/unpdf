//! A content stream is untrusted input: an operator given too few operands (or operands of
//! the wrong type) must be skipped, never panic, and must not stop the text after it.

mod common;

use common::page_pdf;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Every operator of ISO 32000-1 Annex A, plus the `"` and `'` text-showing shorthands.
const OPERATORS: &[&str] = &[
    "b", "B", "b*", "B*", "BDC", "BI", "BMC", "BT", "BX", "c", "cm", "CS", "cs", "d", "d0", "d1",
    "Do", "DP", "EI", "EMC", "ET", "EX", "f", "F", "f*", "G", "g", "gs", "h", "i", "ID", "j", "J",
    "K", "k", "l", "m", "M", "MP", "n", "q", "Q", "re", "RG", "rg", "ri", "s", "S", "SC", "sc",
    "SCN", "scn", "sh", "T*", "Tc", "Td", "TD", "Tf", "Tj", "TJ", "TL", "Tm", "Tr", "Ts", "Tw",
    "Tz", "v", "w", "W", "W*", "y", "'", "\"",
];

/// The operand shapes an operator can be handed by mistake: none at all, and one of the wrong
/// type (a font operator handed a number, a numeric one handed a name).
const WRONG_OPERANDS: &[&str] = &["", "/F1", "12", "(x)", "[1 2]"];

fn content(operands: &str, operator: &str) -> Vec<u8> {
    // Once outside a text object and once inside one, then text that must still come out.
    format!(
        "{operands} {operator}\nBT {operands} {operator} ET\nBT /F1 12 Tf 72 700 Td (Survives) Tj ET"
    )
    .into_bytes()
}

#[test]
fn an_operator_with_missing_or_mistyped_operands_does_not_panic() {
    let mut panicked = Vec::new();
    for operator in OPERATORS {
        for operands in WRONG_OPERANDS {
            let pdf = page_pdf("", "/MediaBox[0 0 595 842]", &content(operands, operator));
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let doc = unpdf::parse_bytes(&pdf)?;
                unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default())
            }));
            if outcome.is_err() {
                panicked.push(format!("`{operands} {operator}`"));
            }
        }
    }
    assert!(
        panicked.is_empty(),
        "parsing panicked on: {}",
        panicked.join(", ")
    );
}

/// Inside a text object, a bad operator is passed over and the text after it is still read —
/// the page is not given up on.
#[test]
fn text_after_an_operator_with_missing_operands_is_still_read() {
    for operator in [
        "Tf", "Td", "TD", "Tm", "Tc", "Tw", "Tz", "TL", "Ts", "Tr", "TJ", "Tj",
    ] {
        let stream = format!("BT {operator} 72 700 Td /F1 12 Tf (Survives) Tj ET");
        let pdf = page_pdf("", "/MediaBox[0 0 595 842]", stream.as_bytes());
        let doc = unpdf::parse_bytes(&pdf).unwrap_or_else(|e| panic!("`{operator}`: {e}"));
        assert!(
            doc.plain_text().contains("Survives"),
            "`{operator}` without operands lost the text after it: {:?}",
            doc.plain_text()
        );
    }
}

#[cfg(feature = "raster")]
#[test]
fn rasterizing_an_operator_with_missing_or_mistyped_operands_does_not_panic() {
    use unpdf::parser::raster::{PageRegion, RasterOptions};
    let options = RasterOptions {
        dpi: 18.0,
        region: PageRegion::Crop,
    };
    let mut panicked = Vec::new();
    for operator in OPERATORS {
        for operands in WRONG_OPERANDS {
            let pdf = page_pdf("", "/MediaBox[0 0 595 842]", &content(operands, operator));
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                unpdf::parser::PdfParser::from_bytes(&pdf)?.render_page(1, &options)
            }));
            if outcome.is_err() {
                panicked.push(format!("`{operands} {operator}`"));
            }
        }
    }
    assert!(
        panicked.is_empty(),
        "rasterizing panicked on: {}",
        panicked.join(", ")
    );
}
