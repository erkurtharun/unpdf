//! `unpdf_open_*_with_options` open a document without extracting it: `unpdf_page_count`
//! answers from the page tree, and the content is extracted the first time something asks
//! for it — with the options given when it was opened.
#![cfg(feature = "ffi")]

mod common;

use std::ffi::{CStr, CString};

use common::distinct_image_per_page_pdf;
use unpdf::ffi::{
    unpdf_free_document, unpdf_free_string, unpdf_last_error_kind, unpdf_open_bytes_with_options,
    unpdf_page_count, unpdf_parse_bytes_with_options, unpdf_section_count, unpdf_to_text,
    UnpdfDocument,
};

fn open(pdf: &[u8], options: Option<&str>) -> *mut UnpdfDocument {
    let options = options.map(|o| CString::new(o).unwrap());
    let ptr = options.as_ref().map_or(std::ptr::null(), |o| o.as_ptr());
    unsafe { unpdf_open_bytes_with_options(pdf.as_ptr(), pdf.len(), ptr) }
}

#[test]
fn an_opened_document_counts_its_pages_and_extracts_on_demand() {
    let pdf = distinct_image_per_page_pdf(3);
    let doc = open(&pdf, None);
    assert!(!doc.is_null());
    unsafe {
        assert_eq!(unpdf_page_count(doc), 3);
        // Asking for content extracts it.
        assert_eq!(unpdf_section_count(doc), 3);
        let text = unpdf_to_text(doc);
        assert!(!text.is_null(), "kind {}", unpdf_last_error_kind());
        let _ = CStr::from_ptr(text);
        unpdf_free_string(text);
        unpdf_free_document(doc);
    }
}

#[test]
fn the_page_count_is_every_page_whatever_the_selection() {
    let pdf = distinct_image_per_page_pdf(3);
    let doc = open(&pdf, Some(r#"{"pages":{"pages":[2]}}"#));
    assert!(!doc.is_null());
    unsafe {
        assert_eq!(unpdf_page_count(doc), 3);
        // The extraction the options select: one page.
        assert_eq!(unpdf_section_count(doc), 1);
        unpdf_free_document(doc);
    }
}

#[test]
fn a_parsed_document_counts_its_pages_too() {
    let pdf = distinct_image_per_page_pdf(2);
    let doc = unsafe { unpdf_parse_bytes_with_options(pdf.as_ptr(), pdf.len(), std::ptr::null()) };
    assert!(!doc.is_null());
    unsafe {
        assert_eq!(unpdf_page_count(doc), 2);
        unpdf_free_document(doc);
    }
}

#[test]
fn bytes_that_are_not_a_pdf_fail_to_open() {
    let doc = open(b"not a pdf at all", None);
    assert!(doc.is_null());
    assert_ne!(unpdf_last_error_kind(), 0);
}
