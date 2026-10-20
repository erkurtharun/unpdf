//! PDF functions of one input (ISO 32000-1 §7.10) — what a shading's colors are made of.
//!
//! Sampled (type 0), exponential (type 2), stitching (type 3) and PostScript calculator
//! (type 4) functions are read. Only a calculator function takes more than one input; a
//! sampled function of several inputs is not read.

use super::postscript::Program;
use super::raw::stream as raw_stream;
use super::raw::tokenizer::{dict_get, PdfDict, PdfObject};
use super::raw::RawDocument;

/// Stitching functions nested deeper than this are not read; the cap bounds a hostile chain.
const MAX_DEPTH: usize = 8;

/// The most samples a sampled function may hold; past it the function is not read.
const MAX_SAMPLES: usize = 1 << 20;

/// A function of one input.
#[derive(Debug, Clone)]
pub(crate) enum Function {
    /// Type 0: a table of samples, interpolated linearly between neighbours.
    Sampled {
        domain: (f32, f32),
        encode: (f32, f32),
        /// Each sample's outputs, already mapped through `Decode`.
        samples: Vec<Vec<f32>>,
    },
    /// Type 2: `C0 + t^N × (C1 − C0)`.
    Exponential {
        domain: (f32, f32),
        c0: Vec<f32>,
        c1: Vec<f32>,
        n: f32,
    },
    /// Type 3: one of `functions` per subdomain between `bounds`, its input mapped by `encode`.
    Stitching {
        domain: (f32, f32),
        functions: Vec<Function>,
        bounds: Vec<f32>,
        encode: Vec<(f32, f32)>,
    },
    /// Type 4: a PostScript calculator program over `domain.len()` inputs, giving
    /// `range.len()` outputs.
    Calculator {
        domain: Vec<(f32, f32)>,
        range: Vec<(f32, f32)>,
        program: Program,
    },
    /// An array of functions, one output each — the form a shading may give instead of one
    /// function of several outputs.
    Each(Vec<Function>),
}

impl Function {
    /// Read the function `obj` (a dictionary, a stream, or an array of them).
    pub(crate) fn read(doc: &RawDocument, obj: &PdfObject) -> Option<Function> {
        read(doc, obj, 0)
    }

    /// How many inputs the function takes.
    pub(crate) fn inputs(&self) -> usize {
        match self {
            Function::Calculator { domain, .. } => domain.len(),
            _ => 1,
        }
    }

    /// The function's outputs at `inputs` (the first, for a function of one input); none when a
    /// calculator program has no result there.
    pub(crate) fn eval_inputs(&self, inputs: &[f32]) -> Vec<f32> {
        match self {
            Function::Calculator {
                domain,
                range,
                program,
            } => {
                if inputs.len() < domain.len() {
                    return Vec::new();
                }
                let clipped: Vec<f32> = inputs
                    .iter()
                    .zip(domain)
                    .map(|(&v, &d)| clip(v, d))
                    .collect();
                match program.run(&clipped) {
                    // The program leaves its outputs on the stack; the top `range.len()` count.
                    Some(stack) if stack.len() >= range.len() => stack[stack.len() - range.len()..]
                        .iter()
                        .zip(range)
                        .map(|(&v, &r)| clip(v, r))
                        .collect(),
                    _ => Vec::new(),
                }
            }
            _ => self.eval(inputs.first().copied().unwrap_or(0.0)),
        }
    }

    /// The function's outputs at `t`.
    pub(crate) fn eval(&self, t: f32) -> Vec<f32> {
        match self {
            Function::Calculator { .. } => self.eval_inputs(&[t]),
            Function::Sampled {
                domain,
                encode,
                samples,
            } => {
                let t = clip(t, *domain);
                let last = (samples.len() - 1) as f32;
                let e = interpolate(t, *domain, *encode).clamp(0.0, last);
                let (i, frac) = (e.floor() as usize, e.fract());
                let a = &samples[i];
                let b = samples.get(i + 1).unwrap_or(a);
                a.iter().zip(b).map(|(&a, &b)| a + (b - a) * frac).collect()
            }
            Function::Exponential { domain, c0, c1, n } => {
                let t = clip(t, *domain);
                let k = t.powf(*n);
                c0.iter().zip(c1).map(|(&a, &b)| a + k * (b - a)).collect()
            }
            Function::Stitching {
                domain,
                functions,
                bounds,
                encode,
            } => {
                let t = clip(t, *domain);
                // The subdomain `t` falls in: bounds[i-1] <= t < bounds[i].
                let i = bounds.iter().take_while(|&&b| t >= b).count();
                let lo = if i == 0 { domain.0 } else { bounds[i - 1] };
                let hi = bounds.get(i).copied().unwrap_or(domain.1);
                let x = interpolate(t, (lo, hi), encode[i]);
                functions[i].eval(x)
            }
            Function::Each(functions) => functions.iter().flat_map(|f| f.eval(t)).collect(),
        }
    }
}

fn clip(t: f32, (lo, hi): (f32, f32)) -> f32 {
    t.clamp(lo.min(hi), lo.max(hi))
}

/// `x` mapped from `from` onto `to` linearly (§7.10.2's `Interpolate`).
fn interpolate(x: f32, from: (f32, f32), to: (f32, f32)) -> f32 {
    if from.1 == from.0 {
        return to.0;
    }
    to.0 + (x - from.0) * (to.1 - to.0) / (from.1 - from.0)
}

fn read(doc: &RawDocument, obj: &PdfObject, depth: usize) -> Option<Function> {
    if depth > MAX_DEPTH {
        return None;
    }
    let obj = doc.resolve(obj);
    if let PdfObject::Array(items) = obj {
        let functions: Option<Vec<Function>> =
            items.iter().map(|f| read(doc, f, depth + 1)).collect();
        return functions.filter(|f| !f.is_empty()).map(Function::Each);
    }
    let (dict, stream) = match obj {
        PdfObject::Dict(dict) => (dict, None),
        PdfObject::Stream(stream) => (&stream.dict, Some(stream)),
        _ => return None,
    };
    let numbers = |key: &[u8]| numbers(doc, dict, key);
    let pairs = |key: &[u8]| numbers(key).map(|n| pairs(&n));
    if dict_get(dict, b"FunctionType")
        .map(|v| doc.resolve(v))
        .and_then(|v| v.as_i64())
        == Some(4)
    {
        let domain = pairs(b"Domain").filter(|d| !d.is_empty())?;
        let range = pairs(b"Range").filter(|r| !r.is_empty())?;
        let program = Program::parse(&raw_stream::decompress(stream?).ok()?)?;
        return Some(Function::Calculator {
            domain,
            range,
            program,
        });
    }
    let domain = *pairs(b"Domain")?.first()?;
    match dict_get(dict, b"FunctionType")
        .map(|v| doc.resolve(v))
        .and_then(|v| v.as_i64())?
    {
        0 => {
            let size = *numbers(b"Size")?.first()? as usize;
            let bits = dict_get(dict, b"BitsPerSample")
                .and_then(|v| doc.resolve(v).as_i64())
                .filter(|b| matches!(b, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32))?
                as usize;
            let range = pairs(b"Range")?;
            let outputs = range.len();
            if size == 0 || outputs == 0 || size.checked_mul(outputs)? > MAX_SAMPLES {
                return None;
            }
            let encode = pairs(b"Encode")
                .and_then(|e| e.first().copied())
                .unwrap_or((0.0, (size - 1) as f32));
            let decode = pairs(b"Decode")
                .filter(|d| d.len() >= outputs)
                .unwrap_or_else(|| range.clone());
            let data = raw_stream::decompress(stream?).ok()?;
            let max = ((1u64 << bits) - 1) as f32;
            let mut bit = 0usize;
            let mut samples = Vec::with_capacity(size);
            for _ in 0..size {
                let mut sample = Vec::with_capacity(outputs);
                for &(lo, hi) in decode.iter().take(outputs) {
                    let raw = read_bits(&data, bit, bits)? as f32;
                    bit += bits;
                    sample.push(lo + raw * (hi - lo) / max);
                }
                samples.push(sample);
            }
            Some(Function::Sampled {
                domain,
                encode,
                samples,
            })
        }
        2 => {
            let c0 = numbers(b"C0").unwrap_or_else(|| vec![0.0]);
            let c1 = numbers(b"C1").unwrap_or_else(|| vec![1.0]);
            let n = dict_get(dict, b"N").and_then(|v| doc.resolve(v).as_f32())?;
            (c0.len() == c1.len()).then_some(Function::Exponential { domain, c0, c1, n })
        }
        3 => {
            let functions: Option<Vec<Function>> = dict_get(dict, b"Functions")
                .map(|v| doc.resolve(v))
                .and_then(|v| v.as_array())?
                .iter()
                .map(|f| read(doc, f, depth + 1))
                .collect();
            let functions = functions?;
            let bounds = numbers(b"Bounds").unwrap_or_default();
            let encode = pairs(b"Encode")?;
            (!functions.is_empty()
                && bounds.len() + 1 == functions.len()
                && encode.len() >= functions.len())
            .then_some(Function::Stitching {
                domain,
                functions,
                bounds,
                encode,
            })
        }
        _ => None,
    }
}

fn numbers(doc: &RawDocument, dict: &PdfDict, key: &[u8]) -> Option<Vec<f32>> {
    doc.resolve(dict_get(dict, key)?)
        .as_array()?
        .iter()
        .map(|v| doc.resolve(v).as_f32())
        .collect()
}

fn pairs(numbers: &[f32]) -> Vec<(f32, f32)> {
    numbers
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[a, b]| (a, b))
        .collect()
}

/// The `bits`-wide unsigned sample starting `bit` bits into `data`, high bits first.
fn read_bits(data: &[u8], bit: usize, bits: usize) -> Option<u32> {
    let mut value: u64 = 0;
    for b in bit..bit + bits {
        let byte = *data.get(b / 8)?;
        value = (value << 1) | u64::from((byte >> (7 - b % 8)) & 1);
    }
    Some(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn an_exponential_function_interpolates_its_two_colors() {
        let f = Function::Exponential {
            domain: (0.0, 1.0),
            c0: vec![1.0, 0.0, 0.0],
            c1: vec![0.0, 0.0, 1.0],
            n: 1.0,
        };
        assert!(close(&f.eval(0.25), &[0.75, 0.0, 0.25]));
        // Outside the domain the input is clipped to it.
        assert!(close(&f.eval(2.0), &[0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_stitching_function_picks_the_subdomain_and_encodes_into_it() {
        let red_to_green = Function::Exponential {
            domain: (0.0, 1.0),
            c0: vec![1.0, 0.0],
            c1: vec![0.0, 1.0],
            n: 1.0,
        };
        let f = Function::Stitching {
            domain: (0.0, 1.0),
            functions: vec![red_to_green.clone(), red_to_green],
            bounds: vec![0.5],
            // The second half runs backwards.
            encode: vec![(0.0, 1.0), (1.0, 0.0)],
        };
        assert!(close(&f.eval(0.25), &[0.5, 0.5]));
        assert!(close(&f.eval(0.5), &[0.0, 1.0]));
        assert!(close(&f.eval(0.75), &[0.5, 0.5]));
    }

    #[test]
    fn a_sampled_function_interpolates_between_samples() {
        let f = Function::Sampled {
            domain: (0.0, 1.0),
            encode: (0.0, 2.0),
            samples: vec![vec![0.0], vec![1.0], vec![0.0]],
        };
        assert!(close(&f.eval(0.25), &[0.5]));
        assert!(close(&f.eval(0.5), &[1.0]));
        assert!(close(&f.eval(1.0), &[0.0]));
    }

    #[test]
    fn a_calculator_function_clips_its_inputs_and_outputs() {
        let f = Function::Calculator {
            domain: vec![(0.0, 1.0), (0.0, 1.0)],
            range: vec![(0.0, 1.0), (0.0, 1.0)],
            program: Program::parse(b"{ 2 mul exch }").unwrap(),
        };
        assert_eq!(f.inputs(), 2);
        // The second input is doubled, then the two are swapped.
        assert!(close(&f.eval_inputs(&[0.25, 0.5]), &[1.0, 0.25]));
        // An input outside its domain is clipped to it, an output outside the range too.
        assert!(close(&f.eval_inputs(&[9.0, 0.5]), &[1.0, 1.0]));
        // Too few inputs: no outputs.
        assert!(f.eval_inputs(&[0.5]).is_empty());
    }

    #[test]
    fn samples_are_read_high_bits_first() {
        assert_eq!(read_bits(&[0b1010_0000], 0, 4), Some(0b1010));
        assert_eq!(read_bits(&[0x12, 0x34], 4, 8), Some(0x23));
        assert_eq!(read_bits(&[0x12], 4, 8), None);
    }
}
