//! A word gap is the room between two glyphs, however a producer makes it — a `TJ`
//! adjustment, or character spacing (`Tc`) that the adjustments inside each word take back.
//!
//! Every glyph of the font below is 500 units wide; text is set at `1 Tf` scaled to 10 pt by
//! the text matrix, so `Tc` is in ems.

mod common;

use common::{assemble, stream_object};

fn monospaced_font() -> Vec<u8> {
    let widths = vec!["500"; 95].join(" ");
    format!("<</Type/Font/Subtype/Type1/BaseFont/Mono/FirstChar 32/LastChar 126/Widths[{widths}]>>")
        .into_bytes()
}

fn text_of(line: &str) -> String {
    let content = format!("BT /F1 1 Tf 10 0 0 10 40 300 Tm {line} ET");
    let pdf = assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 400]/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content.as_bytes()),
        monospaced_font(),
    ]);
    unpdf::parse_bytes(&pdf).expect("parses").plain_text()
}

/// `.381 Tc` puts 0.381 em after every glyph; `375` takes it back inside each word. What is
/// left is the gap between words — and a string of two glyphs (`(es)`) holds a word boundary.
#[test]
fn character_spacing_that_the_adjustments_take_back_inside_words_separates_words() {
    let text =
        text_of(".381 Tc [(t)375(h)375(es)375(e)375(c)375(o)375(n)375(d)-10(o)375(n)375(e)] TJ");
    assert_eq!(text.trim(), "the second one");
}

/// Tracking — a little character spacing over a whole word — is not a word gap.
#[test]
fn light_tracking_does_not_space_a_word_out() {
    assert_eq!(text_of(".05 Tc (TITLE) Tj").trim(), "TITLE");
}

/// Two adjustments in a row are one gap.
#[test]
fn adjacent_adjustments_add_up_to_one_gap() {
    assert_eq!(text_of("[(ab)-70 -70(cd)] TJ").trim(), "ab cd");
}

/// A contents leader set with character spacing is a run of dots, not dots separated by
/// spaces.
#[test]
fn spaced_out_leader_dots_stay_one_run() {
    assert_eq!(text_of(".3 Tc (..........) Tj").trim(), "..........");
}

/// Letter-spaced type has wide room between every two letters; a word gap is room beyond it.
#[test]
fn letter_spaced_words_stay_words() {
    assert_eq!(text_of(".2 Tc [(THE)-333(PLAN)] TJ").trim(), "THE PLAN");
    assert_eq!(
        text_of(".2 Tc [(T)(H)(E)-333(P)(L)(A)(N)] TJ").trim(),
        "THE PLAN"
    );
}

/// Tight tracking pulls letters together; a word gap narrower than a fixed threshold is still
/// a gap against it.
#[test]
fn a_word_gap_in_tightly_tracked_type_is_a_gap() {
    assert_eq!(
        text_of("-.03 Tc [(ALICE)-140(JONES)] TJ").trim(),
        "ALICE JONES"
    );
}
