//! A document whose cross-reference table cannot be used is read from its objects.
//!
//! Each case damages a well-formed document the way files arrive damaged — cut short by a
//! download, offsets shifted by a bad save, the table and trailer lost — and expects the text
//! the intact document gives, with the reconstruction reported in the extraction quality.

mod common;

const LATEX: &[u8] = include_bytes!("fixtures/latex_resume.pdf");

fn text_of(pdf: &[u8]) -> (String, bool) {
    let doc = unpdf::parse_bytes(pdf).expect("the document is read");
    (doc.plain_text(), doc.extraction_quality.xref_reconstructed)
}

/// Where the cross-reference section begins: what `startxref` names.
fn startxref(pdf: &[u8]) -> usize {
    let at = pdf
        .windows(9)
        .rposition(|w| w == b"startxref")
        .expect("startxref");
    let digits: String = pdf[at + 9..]
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(|b| b.is_ascii_digit())
        .map(|&b| b as char)
        .collect();
    digits.parse().expect("offset")
}

fn documents() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("text_pdf", common::text_pdf()),
        ("two_column_pdf", common::two_column_pdf()),
        ("latex_resume (cross-reference stream)", LATEX.to_vec()),
    ]
}

#[test]
fn an_intact_document_is_not_reported_as_reconstructed() {
    for (name, pdf) in documents() {
        let (text, reconstructed) = text_of(&pdf);
        assert!(!text.is_empty(), "{name}");
        assert!(!reconstructed, "{name}");
    }
}

/// Cut short after its last object: no cross-reference section, no trailer, no `startxref`.
#[test]
fn a_document_cut_after_its_objects_reads_as_the_whole() {
    for (name, pdf) in documents() {
        let (whole, _) = text_of(&pdf);
        let cut = &pdf[..startxref(&pdf)];
        let (text, reconstructed) = text_of(cut);
        assert_eq!(text, whole, "{name}");
        assert!(reconstructed, "{name}");
    }
}

/// `startxref` names an offset where no cross-reference section is.
#[test]
fn a_wrong_startxref_offset_is_recovered_from() {
    for (name, pdf) in documents() {
        let (whole, _) = text_of(&pdf);
        let at = pdf.windows(9).rposition(|w| w == b"startxref").unwrap();
        let mut damaged = pdf[..at].to_vec();
        damaged.extend_from_slice(b"startxref\n17\n%%EOF\n");
        let (text, reconstructed) = text_of(&damaged);
        assert_eq!(text, whole, "{name}");
        assert!(reconstructed, "{name}");
    }
}

/// Bytes added after the header shift every object, so every offset in the table is wrong —
/// read as written, each would land inside another object.
#[test]
fn offsets_shifted_by_a_bad_save_are_recovered_from() {
    let pdf = common::text_pdf();
    let (whole, _) = text_of(&pdf);
    let header_end = pdf.iter().position(|&b| b == b'\n').unwrap() + 1;
    let mut shifted = pdf[..header_end].to_vec();
    shifted.extend_from_slice(b"%padding written by a broken editor\n");
    shifted.extend_from_slice(&pdf[header_end..]);
    let (text, reconstructed) = text_of(&shifted);
    assert_eq!(text, whole);
    assert!(reconstructed);
}

/// With no trailer anywhere, the catalog is found by its `/Type`.
#[test]
fn without_a_trailer_the_catalog_is_found_by_its_type() {
    let pdf = common::text_pdf();
    let (whole, _) = text_of(&pdf);
    let xref = pdf.windows(4).rposition(|w| w == b"xref").unwrap();
    let (text, reconstructed) = text_of(&pdf[..xref]);
    assert_eq!(text, whole);
    assert!(reconstructed);
}

/// A file with no objects to rebuild from still fails, with the original error.
#[test]
fn a_file_with_nothing_to_rebuild_from_is_still_an_error() {
    assert!(unpdf::parse_bytes(b"%PDF-1.4\nnot a document at all\n").is_err());
}

/// A `/Prev` pointing back at its own section is read once — it used to loop forever.
#[test]
fn a_prev_chain_that_loops_ends() {
    let pdf = common::text_pdf();
    let xref = startxref(&pdf);
    let trailer = pdf.windows(7).rposition(|w| w == b"trailer").unwrap();
    let dict_start = trailer + pdf[trailer..].windows(2).position(|w| w == b"<<").unwrap() + 2;
    let mut looped = pdf[..dict_start].to_vec();
    looped.extend_from_slice(format!("/Prev {xref} ").as_bytes());
    looped.extend_from_slice(&pdf[dict_start..]);
    let (text, _) = text_of(&looped);
    assert_eq!(text, text_of(&pdf).0);
}
