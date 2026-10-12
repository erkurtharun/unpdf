//! Whole documents: the file structure — header, cross-reference tables and streams, object
//! streams, encryption — then every page's content and the Markdown render.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(doc) = unpdf::parse_bytes(data) {
        let _ = unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default());
    }
});
