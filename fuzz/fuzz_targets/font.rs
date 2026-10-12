//! Embedded fonts and CMaps: the fuzzed bytes become the font program (TrueType, CFF, Type 1)
//! or the `/ToUnicode` CMap of the font a line of text is drawn with. The first byte chooses.
#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "pdf.rs"]
mod pdf;

const CONTENT: &[u8] = b"BT /F1 12 Tf 20 200 Td (Hello) Tj <0041> Tj ET";

fuzz_target!(|data: &[u8]| {
    let Some((&choice, bytes)) = data.split_first() else {
        return;
    };
    let descriptor = |file_key: &str| {
        format!(
            "<</Type/FontDescriptor/FontName/Fuzz/Flags 32/FontBBox[0 0 1000 1000]/ItalicAngle 0\
             /Ascent 800/Descent -200/CapHeight 700/StemV 80/{file_key} 7 0 R>>"
        )
        .into_bytes()
    };
    let (font, extra) = match choice % 4 {
        0 => (
            b"<</Type/Font/Subtype/TrueType/BaseFont/Fuzz/FontDescriptor 6 0 R>>".to_vec(),
            vec![descriptor("FontFile2"), pdf::stream("", bytes)],
        ),
        1 => (
            b"<</Type/Font/Subtype/Type1/BaseFont/Fuzz/FontDescriptor 6 0 R>>".to_vec(),
            vec![
                descriptor("FontFile3"),
                pdf::stream("/Subtype/Type1C", bytes),
            ],
        ),
        2 => (
            b"<</Type/Font/Subtype/Type1/BaseFont/Fuzz/FontDescriptor 6 0 R>>".to_vec(),
            vec![descriptor("FontFile"), pdf::stream("", bytes)],
        ),
        _ => (
            b"<</Type/Font/Subtype/Type0/BaseFont/Fuzz/Encoding/Identity-H\
              /DescendantFonts[6 0 R]/ToUnicode 7 0 R>>"
                .to_vec(),
            vec![
                b"<</Type/Font/Subtype/CIDFontType2/BaseFont/Fuzz\
                  /CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>>>"
                    .to_vec(),
                pdf::stream("", bytes),
            ],
        ),
    };
    let doc = pdf::page(CONTENT, font, extra);
    if let Ok(parsed) = unpdf::parse_bytes(&doc) {
        let _ = unpdf::render::to_markdown(&parsed, &unpdf::RenderOptions::default());
    }
});
