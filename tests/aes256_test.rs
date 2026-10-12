//! AES-256 encryption — the standard security handler's revisions 5 and 6.
//!
//! The fixtures are written by `tests/fixtures/make_aes256_fixtures.py`: the revision 6 files
//! by PyMuPDF (an independent writer, so these check the key derivation against someone
//! else's), the revision 5 file from the specification with `hashlib` and `cryptography`.

use unpdf::{Error, ParseOptions};

const R6_OPEN: &[u8] = include_bytes!("fixtures/aes256-r6-open.pdf");
const R6_USER: &[u8] = include_bytes!("fixtures/aes256-r6-user.pdf");
const R5: &[u8] = include_bytes!("fixtures/aes256-r5.pdf");
const TEXT: &str = "Encrypted with AES-256";

fn read(pdf: &[u8], password: Option<&str>) -> unpdf::Result<unpdf::Document> {
    let mut options = ParseOptions::new();
    if let Some(password) = password {
        options = options.with_password(password);
    }
    unpdf::parse_bytes_with_options(pdf, options)
}

#[test]
fn a_revision_6_document_with_an_empty_user_password_opens_without_one() {
    let doc = read(R6_OPEN, None).expect("opens with the empty user password");
    assert_eq!(doc.plain_text().trim(), TEXT);
    assert!(doc.extraction_quality.encrypted);
}

#[test]
fn a_revision_6_document_opens_with_its_user_or_owner_password() {
    for password in ["user", "owner"] {
        let doc = read(R6_USER, Some(password)).unwrap_or_else(|e| panic!("{password}: {e}"));
        assert_eq!(doc.plain_text().trim(), TEXT, "{password}");
    }
}

#[test]
fn a_revision_5_document_opens_with_its_user_or_owner_password() {
    for password in ["user", "owner"] {
        let doc = read(R5, Some(password)).unwrap_or_else(|e| panic!("{password}: {e}"));
        assert_eq!(doc.plain_text().trim(), TEXT, "{password}");
    }
}

/// Without a password the document asks for one; with a wrong one it says the password is
/// wrong — two different things for a caller to do.
#[test]
fn a_missing_or_wrong_password_is_reported_as_such() {
    for pdf in [R6_USER, R5] {
        assert!(matches!(read(pdf, None), Err(Error::Encrypted)));
        assert!(matches!(
            read(pdf, Some("nope")),
            Err(Error::InvalidPassword)
        ));
    }
}

/// MuPDF writes the `/Encrypt` dictionary into the trailer instead of referencing it. That is
/// allowed, and it was refused as an unparsable encryption dictionary at every revision — so
/// this revision 4 (AES-128) file pins the same fix outside AES-256.
#[test]
fn an_encrypt_dictionary_written_into_the_trailer_is_read() {
    const AES128: &[u8] = include_bytes!("fixtures/aes128-mupdf.pdf");
    let doc = read(AES128, None).expect("opens with the empty user password");
    assert_eq!(doc.plain_text().trim(), TEXT);
}
