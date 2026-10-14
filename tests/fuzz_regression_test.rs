//! Inputs the fuzzer found, kept as fixtures: `tests/fixtures/fuzz-<what it broke>.pdf`. Each
//! must parse and render, or fail with an error -- never panic. (The fuzz workflow also seeds
//! its `load` corpus from `tests/fixtures/*.pdf`, so a finding stays in the corpus.)

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

#[test]
fn fuzz_findings_parse_or_fail_cleanly() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let findings: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/fixtures")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("fuzz-") && name.ends_with(".pdf"))
        })
        .collect();
    assert!(!findings.is_empty(), "no fuzz-*.pdf fixture in {dir:?}");
    for path in findings {
        let data = std::fs::read(&path).expect("a readable fixture");
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            if let Ok(doc) = unpdf::parse_bytes(&data) {
                let _ = unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default());
            }
        }));
        assert!(outcome.is_ok(), "{path:?} panicked");
    }
}
