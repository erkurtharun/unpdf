//! BiDi (bidirectional) text reordering for RTL script support.

use unicode_bidi::BidiInfo;
use unicode_bidi_mirroring::get_mirrored;
use unicode_normalization::UnicodeNormalization;

/// Check if text contains RTL characters (Arabic, Hebrew, etc.)
pub fn contains_rtl(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{0590}'..='\u{05FF}' |  // Hebrew
            '\u{0600}'..='\u{06FF}' |  // Arabic
            '\u{0700}'..='\u{074F}' |  // Syriac
            '\u{0750}'..='\u{077F}' |  // Arabic Supplement
            '\u{0780}'..='\u{07BF}' |  // Thaana
            '\u{07C0}'..='\u{07FF}' |  // NKo
            '\u{0800}'..='\u{083F}' |  // Samaritan
            '\u{0840}'..='\u{085F}' |  // Mandaic
            '\u{08A0}'..='\u{08FF}' |  // Arabic Extended-A
            '\u{FB50}'..='\u{FDFF}' |  // Arabic Presentation Forms-A
            '\u{FE70}'..='\u{FEFF}'    // Arabic Presentation Forms-B
        )
    })
}

/// Apply BiDi reordering to convert from visual to logical order,
/// mirroring the characters of right-to-left runs (UAX #9 L4), then
/// unfold the right-to-left presentation forms it holds to their base letters.
pub fn reorder_bidi(text: &str) -> String {
    if !contains_rtl(text) {
        return text.to_string();
    }

    let bidi_info = BidiInfo::new(text, None);
    let mut result = String::new();

    for para in &bidi_info.paragraphs {
        let (levels, runs) = bidi_info.visual_runs(para, para.range.clone());
        for run in runs {
            if levels[run.start].is_rtl() {
                result.extend(
                    text[run]
                        .chars()
                        .rev()
                        .map(|c| get_mirrored(c).unwrap_or(c)),
                );
            } else {
                result.push_str(&text[run]);
            }
        }
    }

    unfold_presentation_forms(&result)
}

/// Whether `c` is a presentation form of a right-to-left script: Hebrew (U+FB1D–U+FB4F) or
/// Arabic, forms A (U+FB50–U+FDFF) and B (U+FE70–U+FEFF). A font draws a shaped letter as one
/// and the text stands for its base letters.
fn is_rtl_presentation_form(c: char) -> bool {
    matches!(c, '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
}

/// `text` with each right-to-left presentation form replaced by its compatibility
/// decomposition (NFKC), and every other character as it is. Folding the whole line would
/// also turn `²` into `2`, `ﬁ` into `fi` and full-width letters into ASCII in the parts of the
/// line that are not Arabic or Hebrew.
fn unfold_presentation_forms(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut buf = [0u8; 4];
    for c in text.chars() {
        if is_rtl_presentation_form(c) {
            out.extend(c.encode_utf8(&mut buf).nfkc());
        } else {
            out.push(c);
        }
    }
    out
}
