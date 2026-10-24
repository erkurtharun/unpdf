use unpdf::parser::bidi;

#[test]
fn test_contains_rtl_arabic() {
    assert!(bidi::contains_rtl("مرحبا"));
    assert!(bidi::contains_rtl("Hello مرحبا World"));
}

#[test]
fn test_contains_rtl_hebrew() {
    assert!(bidi::contains_rtl("שלום"));
}

#[test]
fn test_contains_rtl_latin() {
    assert!(!bidi::contains_rtl("Hello World"));
    assert!(!bidi::contains_rtl("123 abc"));
}

#[test]
fn test_reorder_bidi_latin() {
    // Latin text should pass through unchanged
    let result = bidi::reorder_bidi("Hello World");
    assert_eq!(result, "Hello World");
}

#[test]
fn test_reorder_bidi_arabic() {
    // Just verify it doesn't panic and returns non-empty
    let result = bidi::reorder_bidi("مرحبا");
    assert!(!result.is_empty());
}

#[test]
fn test_reorder_bidi_mirrors_brackets_in_rtl_run() {
    assert_eq!(bidi::reorder_bidi("ج (ب) ا"), "ا (ب) ج");
}

/// A guard, not a regression test: it passes with or without the mirroring -- brackets in a
/// left-to-right run must stay as drawn.
#[test]
fn test_reorder_bidi_keeps_brackets_in_ltr_run() {
    assert_eq!(bidi::reorder_bidi("abc (d) ج"), "abc (d) ج");
}

/// Only the right-to-left presentation forms are unfolded: a superscript two, a ligature
/// and a full-width letter beside Arabic text stay as they are.
#[test]
fn test_reorder_bidi_unfolds_only_rtl_presentation_forms() {
    // U+FE8E is ARABIC LETTER ALEF FINAL FORM, which unfolds to U+0627.
    let line = "m\u{00B2} \u{FB01} \u{FF21} \u{FE8E}";
    let out = bidi::reorder_bidi(line);
    assert!(out.contains("m\u{00B2}"), "{out:?}");
    assert!(out.contains('\u{FB01}'), "{out:?}");
    assert!(out.contains('\u{FF21}'), "{out:?}");
    assert!(
        out.contains('\u{0627}') && !out.contains('\u{FE8E}'),
        "{out:?}"
    );
}
