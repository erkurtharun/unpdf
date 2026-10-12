//! Damaged input never panics: a fixed, seeded set of mutations of real documents — bytes
//! flipped, files cut short, ranges zeroed or repeated — parsed and rendered on every test run.
//!
//! This is the stable-toolchain floor under the `fuzz/` targets (`cargo fuzz`, nightly), which
//! explore far more. A panic found by either belongs here as a fixed case once it is fixed.

mod common;

use std::panic::{catch_unwind, AssertUnwindSafe};

const LATEX: &[u8] = include_bytes!("fixtures/latex_resume.pdf");
const AES256: &[u8] = include_bytes!("fixtures/aes256-r6-open.pdf");
const MUTATIONS_PER_SEED: usize = 60;

/// xorshift64*: a deterministic stream, so a failure names a case that reproduces.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(data: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = data.to_vec();
    match rng.below(4) {
        0 => {
            for _ in 0..1 + rng.below(20) {
                let i = rng.below(b.len());
                b[i] = rng.next() as u8;
            }
        }
        1 => b.truncate(b.len() / 4 + rng.below(b.len() * 3 / 4)),
        2 => {
            let i = rng.below(b.len());
            let end = (i + 1 + rng.below(512)).min(b.len());
            b[i..end].fill(0);
        }
        _ => {
            let i = rng.below(b.len());
            let end = (i + 1 + rng.below(2048)).min(b.len());
            let copy = b[i..end].to_vec();
            b.splice(i..i, copy);
        }
    }
    b
}

#[test]
fn mutated_documents_never_panic() {
    let seeds: Vec<(&str, Vec<u8>)> = vec![
        ("text_pdf", common::text_pdf()),
        ("two_column_pdf", common::two_column_pdf()),
        ("bordered_table_pdf", common::bordered_table_pdf()),
        ("latex_resume", LATEX.to_vec()),
        ("aes256-r6-open", AES256.to_vec()),
    ];
    let mut panicked = Vec::new();
    for (name, seed) in &seeds {
        let mut rng = Rng(0x5EED_0000 ^ seed.len() as u64);
        for case in 0..MUTATIONS_PER_SEED {
            let damaged = mutate(seed, &mut rng);
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                if let Ok(doc) = unpdf::parse_bytes(&damaged) {
                    let _ = unpdf::render::to_markdown(&doc, &unpdf::RenderOptions::default());
                    let _ = unpdf::render::to_text(&doc, &unpdf::RenderOptions::default());
                }
            }));
            if outcome.is_err() {
                panicked.push(format!("{name} case {case}"));
            }
        }
    }
    assert!(panicked.is_empty(), "panicked on: {}", panicked.join(", "));
}
