//! Content streams: the operators and operands of a page — text state, text showing, paths,
//! images, inline images, marked content — through parsing, Markdown and rasterization.
#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "pdf.rs"]
mod pdf;

fuzz_target!(|data: &[u8]| {
    let doc = pdf::page(
        data,
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
        Vec::new(),
    );
    if let Ok(parsed) = unpdf::parse_bytes(&doc) {
        let _ = unpdf::render::to_markdown(&parsed, &unpdf::RenderOptions::default());
    }
    if let Ok(parser) = unpdf::parser::PdfParser::from_bytes(&doc) {
        let options = unpdf::parser::raster::RasterOptions {
            dpi: 12.0,
            region: unpdf::parser::raster::PageRegion::Crop,
        };
        let _ = parser.render_page(1, &options);
    }
});
