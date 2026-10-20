//! PostScript calculator functions (ISO 32000-1 §7.10.5, function type 4).
//!
//! A type 4 function is a small stack program in braces: numbers push themselves, operators
//! pop their operands and push results, and `if` / `ifelse` run a braced procedure. It is how
//! a `DeviceN` or `Separation` space most often says what its tints look like on the press.
//! The program is parsed once and run once per color; a program that is malformed, runs out of
//! operands, mixes types, or runs past [`MAX_STEPS`] has no result.

/// Operations one run may execute; past it the run has no result. Bounds a hostile loop-free
/// program's cost (`roll` and `copy` repeat work, and a color runs it once per pixel).
const MAX_STEPS: usize = 10_000;

/// The deepest operand stack (§7.10.5, Annex C: 100).
const MAX_STACK: usize = 100;

/// Procedures nested deeper than this are not read.
const MAX_NESTING: usize = 16;

/// A value on the stack.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Val {
    Int(i32),
    Real(f32),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Abs,
    Add,
    Atan,
    Ceiling,
    Cos,
    Cvi,
    Cvr,
    Div,
    Exp,
    Floor,
    Idiv,
    Ln,
    Log,
    Mod,
    Mul,
    Neg,
    Round,
    Sin,
    Sqrt,
    Sub,
    Truncate,
    And,
    Bitshift,
    Eq,
    False,
    Ge,
    Gt,
    Le,
    Lt,
    Ne,
    Not,
    Or,
    True,
    Xor,
    Copy,
    Dup,
    Exch,
    Index,
    Pop,
    Roll,
}

impl Op {
    fn named(name: &str) -> Option<Op> {
        Some(match name {
            "abs" => Op::Abs,
            "add" => Op::Add,
            "atan" => Op::Atan,
            "ceiling" => Op::Ceiling,
            "cos" => Op::Cos,
            "cvi" => Op::Cvi,
            "cvr" => Op::Cvr,
            "div" => Op::Div,
            "exp" => Op::Exp,
            "floor" => Op::Floor,
            "idiv" => Op::Idiv,
            "ln" => Op::Ln,
            "log" => Op::Log,
            "mod" => Op::Mod,
            "mul" => Op::Mul,
            "neg" => Op::Neg,
            "round" => Op::Round,
            "sin" => Op::Sin,
            "sqrt" => Op::Sqrt,
            "sub" => Op::Sub,
            "truncate" => Op::Truncate,
            "and" => Op::And,
            "bitshift" => Op::Bitshift,
            "eq" => Op::Eq,
            "false" => Op::False,
            "ge" => Op::Ge,
            "gt" => Op::Gt,
            "le" => Op::Le,
            "lt" => Op::Lt,
            "ne" => Op::Ne,
            "not" => Op::Not,
            "or" => Op::Or,
            "true" => Op::True,
            "xor" => Op::Xor,
            "copy" => Op::Copy,
            "dup" => Op::Dup,
            "exch" => Op::Exch,
            "index" => Op::Index,
            "pop" => Op::Pop,
            "roll" => Op::Roll,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
enum Instr {
    Push(Val),
    Op(Op),
    If(Vec<Instr>),
    IfElse(Vec<Instr>, Vec<Instr>),
}

/// A parsed calculator program.
#[derive(Debug, Clone)]
pub(crate) struct Program(Vec<Instr>);

/// What the parser has so far in one block: instructions, or a braced procedure waiting for
/// the `if` / `ifelse` that takes it.
enum Item {
    Instr(Instr),
    Block(Vec<Instr>),
}

impl Program {
    /// Parse `source`: one braced procedure.
    pub(crate) fn parse(source: &[u8]) -> Option<Program> {
        let text = std::str::from_utf8(source).ok()?;
        let tokens = tokenize(text);
        let mut pos = 0;
        if tokens.first() != Some(&"{") {
            return None;
        }
        pos += 1;
        let body = parse_block(&tokens, &mut pos, 0)?;
        (pos == tokens.len()).then_some(Program(body))
    }

    /// The program's results on `inputs`, bottom of the stack first; `None` when the run has
    /// none.
    pub(crate) fn run(&self, inputs: &[f32]) -> Option<Vec<f32>> {
        let mut stack: Vec<Val> = inputs.iter().map(|&v| Val::Real(v)).collect();
        let mut steps = 0usize;
        exec(&self.0, &mut stack, &mut steps)?;
        stack
            .into_iter()
            .map(|v| match v {
                Val::Int(i) => Some(i as f32),
                Val::Real(r) => Some(r),
                Val::Bool(_) => None,
            })
            .collect()
    }
}

/// Split into braces and words; `%` starts a comment to the end of its line.
fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        match c {
            '%' => {
                for (_, c) in chars.by_ref() {
                    if c == '\n' || c == '\r' {
                        break;
                    }
                }
            }
            '{' | '}' => {
                tokens.push(&text[start..start + 1]);
                chars.next();
            }
            c if c.is_whitespace() || c == '\0' => {
                chars.next();
            }
            _ => {
                let mut end = start;
                while let Some(&(i, c)) = chars.peek() {
                    if c.is_whitespace() || matches!(c, '{' | '}' | '%' | '\0') {
                        break;
                    }
                    end = i + c.len_utf8();
                    chars.next();
                }
                tokens.push(&text[start..end]);
            }
        }
    }
    tokens
}

/// The instructions up to the `}` that closes the block `pos` is inside.
fn parse_block(tokens: &[&str], pos: &mut usize, depth: usize) -> Option<Vec<Instr>> {
    if depth > MAX_NESTING {
        return None;
    }
    let mut items: Vec<Item> = Vec::new();
    loop {
        let token = *tokens.get(*pos)?;
        *pos += 1;
        match token {
            "}" => break,
            "{" => items.push(Item::Block(parse_block(tokens, pos, depth + 1)?)),
            "if" => {
                let Some(Item::Block(body)) = items.pop() else {
                    return None;
                };
                items.push(Item::Instr(Instr::If(body)));
            }
            "ifelse" => {
                let (Some(Item::Block(otherwise)), Some(Item::Block(then))) =
                    (items.pop(), items.pop())
                else {
                    return None;
                };
                items.push(Item::Instr(Instr::IfElse(then, otherwise)));
            }
            word => items.push(Item::Instr(match Op::named(word) {
                Some(op) => Instr::Op(op),
                None => Instr::Push(number(word)?),
            })),
        }
    }
    items
        .into_iter()
        .map(|item| match item {
            Item::Instr(instr) => Some(instr),
            Item::Block(_) => None, // a procedure no `if` takes
        })
        .collect()
}

fn number(word: &str) -> Option<Val> {
    if let Ok(i) = word.parse::<i32>() {
        return Some(Val::Int(i));
    }
    word.parse::<f32>()
        .ok()
        .filter(|r| r.is_finite())
        .map(Val::Real)
}

fn exec(code: &[Instr], stack: &mut Vec<Val>, steps: &mut usize) -> Option<()> {
    for instr in code {
        *steps += 1;
        if *steps > MAX_STEPS || stack.len() > MAX_STACK {
            return None;
        }
        match instr {
            Instr::Push(v) => stack.push(*v),
            Instr::Op(op) => apply(*op, stack)?,
            Instr::If(body) => {
                if pop_bool(stack)? {
                    exec(body, stack, steps)?;
                }
            }
            Instr::IfElse(then, otherwise) => {
                let branch = if pop_bool(stack)? { then } else { otherwise };
                exec(branch, stack, steps)?;
            }
        }
    }
    Some(())
}

fn pop_bool(stack: &mut Vec<Val>) -> Option<bool> {
    match stack.pop()? {
        Val::Bool(b) => Some(b),
        _ => None,
    }
}

fn pop_int(stack: &mut Vec<Val>) -> Option<i32> {
    match stack.pop()? {
        Val::Int(i) => Some(i),
        _ => None,
    }
}

fn as_f32(v: Val) -> Option<f32> {
    match v {
        Val::Int(i) => Some(i as f32),
        Val::Real(r) => Some(r),
        Val::Bool(_) => None,
    }
}

fn pop_num(stack: &mut Vec<Val>) -> Option<f32> {
    as_f32(stack.pop()?)
}

/// A real result, kept finite: an overflow or a NaN is no result.
fn real(v: f32) -> Option<Val> {
    v.is_finite().then_some(Val::Real(v))
}

fn apply(op: Op, stack: &mut Vec<Val>) -> Option<()> {
    match op {
        Op::Add | Op::Sub | Op::Mul => {
            let b = stack.pop()?;
            let a = stack.pop()?;
            let result = match (a, b) {
                (Val::Int(x), Val::Int(y)) => match op {
                    Op::Add => x.checked_add(y),
                    Op::Sub => x.checked_sub(y),
                    _ => x.checked_mul(y),
                }
                .map(Val::Int)
                .or_else(|| {
                    let (x, y) = (x as f32, y as f32);
                    real(match op {
                        Op::Add => x + y,
                        Op::Sub => x - y,
                        _ => x * y,
                    })
                })?,
                _ => {
                    let (x, y) = (as_f32(a)?, as_f32(b)?);
                    real(match op {
                        Op::Add => x + y,
                        Op::Sub => x - y,
                        _ => x * y,
                    })?
                }
            };
            stack.push(result);
        }
        Op::Div => {
            let b = pop_num(stack)?;
            let a = pop_num(stack)?;
            stack.push(real(a / b)?);
        }
        Op::Idiv => {
            let b = pop_int(stack)?;
            let a = pop_int(stack)?;
            stack.push(Val::Int(a.checked_div(b)?));
        }
        Op::Mod => {
            let b = pop_int(stack)?;
            let a = pop_int(stack)?;
            stack.push(Val::Int(a.checked_rem(b)?));
        }
        Op::Abs | Op::Neg | Op::Ceiling | Op::Floor | Op::Round | Op::Truncate => {
            let v = stack.pop()?;
            stack.push(match v {
                Val::Int(i) => match op {
                    Op::Abs => Val::Int(i.checked_abs()?),
                    Op::Neg => Val::Int(i.checked_neg()?),
                    _ => Val::Int(i),
                },
                Val::Real(r) => real(match op {
                    Op::Abs => r.abs(),
                    Op::Neg => -r,
                    Op::Ceiling => r.ceil(),
                    Op::Floor => r.floor(),
                    // PostScript rounds halves up.
                    Op::Round => (r + 0.5).floor(),
                    _ => r.trunc(),
                })?,
                Val::Bool(_) => return None,
            });
        }
        Op::Sqrt => {
            let v = pop_num(stack)?;
            (v >= 0.0).then_some(())?;
            stack.push(real(v.sqrt())?);
        }
        Op::Sin | Op::Cos => {
            let degrees = pop_num(stack)?;
            let radians = degrees.to_radians();
            stack.push(real(if op == Op::Sin {
                radians.sin()
            } else {
                radians.cos()
            })?);
        }
        Op::Atan => {
            let den = pop_num(stack)?;
            let num = pop_num(stack)?;
            (num != 0.0 || den != 0.0).then_some(())?;
            let mut degrees = num.atan2(den).to_degrees();
            if degrees < 0.0 {
                degrees += 360.0;
            }
            stack.push(real(degrees)?);
        }
        Op::Exp => {
            let exponent = pop_num(stack)?;
            let base = pop_num(stack)?;
            stack.push(real(base.powf(exponent))?);
        }
        Op::Ln | Op::Log => {
            let v = pop_num(stack)?;
            (v > 0.0).then_some(())?;
            stack.push(real(if op == Op::Ln { v.ln() } else { v.log10() })?);
        }
        Op::Cvi => {
            let v = pop_num(stack)?.trunc();
            (v.abs() < i32::MAX as f32).then_some(())?;
            stack.push(Val::Int(v as i32));
        }
        Op::Cvr => {
            let v = pop_num(stack)?;
            stack.push(Val::Real(v));
        }
        Op::And | Op::Or | Op::Xor => {
            let b = stack.pop()?;
            let a = stack.pop()?;
            stack.push(match (a, b) {
                (Val::Bool(x), Val::Bool(y)) => Val::Bool(match op {
                    Op::And => x & y,
                    Op::Or => x | y,
                    _ => x ^ y,
                }),
                (Val::Int(x), Val::Int(y)) => Val::Int(match op {
                    Op::And => x & y,
                    Op::Or => x | y,
                    _ => x ^ y,
                }),
                _ => return None,
            });
        }
        Op::Not => {
            let v = stack.pop()?;
            stack.push(match v {
                Val::Bool(b) => Val::Bool(!b),
                Val::Int(i) => Val::Int(!i),
                Val::Real(_) => return None,
            });
        }
        Op::Bitshift => {
            let shift = pop_int(stack)?;
            let v = pop_int(stack)?;
            stack.push(Val::Int(if shift >= 0 {
                v.checked_shl(shift as u32)?
            } else {
                v.checked_shr(shift.unsigned_abs())?
            }));
        }
        Op::Eq | Op::Ne => {
            let b = stack.pop()?;
            let a = stack.pop()?;
            let equal = match (a, b) {
                (Val::Bool(x), Val::Bool(y)) => x == y,
                (Val::Bool(_), _) | (_, Val::Bool(_)) => return None,
                _ => as_f32(a)? == as_f32(b)?,
            };
            stack.push(Val::Bool(equal == (op == Op::Eq)));
        }
        Op::Gt | Op::Ge | Op::Lt | Op::Le => {
            let b = pop_num(stack)?;
            let a = pop_num(stack)?;
            stack.push(Val::Bool(match op {
                Op::Gt => a > b,
                Op::Ge => a >= b,
                Op::Lt => a < b,
                _ => a <= b,
            }));
        }
        Op::True => stack.push(Val::Bool(true)),
        Op::False => stack.push(Val::Bool(false)),
        Op::Pop => {
            stack.pop()?;
        }
        Op::Dup => {
            let v = *stack.last()?;
            stack.push(v);
        }
        Op::Exch => {
            let n = stack.len();
            (n >= 2).then_some(())?;
            stack.swap(n - 1, n - 2);
        }
        Op::Copy => {
            let n = usize::try_from(pop_int(stack)?).ok()?;
            (n <= stack.len() && stack.len() + n <= MAX_STACK).then_some(())?;
            let from = stack.len() - n;
            stack.extend_from_within(from..);
        }
        Op::Index => {
            let n = usize::try_from(pop_int(stack)?).ok()?;
            let v = *stack.get(stack.len().checked_sub(n + 1)?)?;
            stack.push(v);
        }
        Op::Roll => {
            let j = pop_int(stack)?;
            let n = usize::try_from(pop_int(stack)?).ok()?;
            (n <= stack.len()).then_some(())?;
            if n > 0 {
                let from = stack.len() - n;
                // A positive `j` moves the top elements to the bottom of the `n`.
                let by = j.rem_euclid(n as i32) as usize;
                stack[from..].rotate_right(by);
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, inputs: &[f32]) -> Option<Vec<f32>> {
        Program::parse(source.as_bytes())?.run(inputs)
    }

    #[test]
    fn arithmetic_keeps_integers_integer_and_div_makes_reals() {
        assert_eq!(run("{ 3 4 add }", &[]), Some(vec![7.0]));
        assert_eq!(run("{ 7 2 idiv 7 2 mod }", &[]), Some(vec![3.0, 1.0]));
        assert_eq!(run("{ 1 4 div }", &[]), Some(vec![0.25]));
        assert_eq!(run("{ 2 3 exp }", &[]), Some(vec![8.0]));
        assert_eq!(run("{ 2.5 round -2.5 round }", &[]), Some(vec![3.0, -2.0]));
    }

    #[test]
    fn the_stack_operators_follow_postscript() {
        assert_eq!(run("{ 1 2 exch }", &[]), Some(vec![2.0, 1.0]));
        assert_eq!(run("{ 1 2 3 3 1 roll }", &[]), Some(vec![3.0, 1.0, 2.0]));
        assert_eq!(run("{ 1 2 3 3 -1 roll }", &[]), Some(vec![2.0, 3.0, 1.0]));
        assert_eq!(
            run("{ 5 6 7 2 index }", &[]),
            Some(vec![5.0, 6.0, 7.0, 5.0])
        );
        assert_eq!(run("{ 1 2 2 copy }", &[]), Some(vec![1.0, 2.0, 1.0, 2.0]));
        assert_eq!(run("{ 9 dup pop }", &[]), Some(vec![9.0]));
    }

    #[test]
    fn conditionals_run_the_branch_the_test_picks() {
        assert_eq!(
            run("{ dup 0.5 gt { pop 1 } { pop 0 } ifelse }", &[0.9]),
            Some(vec![1.0])
        );
        assert_eq!(
            run("{ dup 0.5 gt { pop 1 } { pop 0 } ifelse }", &[0.1]),
            Some(vec![0.0])
        );
        assert_eq!(
            run("{ dup 0.5 gt { 2 mul } if }", &[0.25]),
            Some(vec![0.25])
        );
        assert_eq!(run("{ dup 0.5 gt { 2 mul } if }", &[0.75]), Some(vec![1.5]));
    }

    #[test]
    fn a_black_only_devicen_transform_from_a_real_file_maps_the_tint_to_k() {
        // The tint transform of a one-colorant `DeviceN [/Black]` over DeviceCMYK, as a layout
        // program writes it: the tint ends up as K, the other three inks zero.
        let program = concat!(
            "{1.000000 2 1 roll 1.000000 2 1 roll 1.000000 2 1 roll 0 index 1.000000 ",
            "cvr exch sub 2 1 roll 5 -1 roll 1.000000 cvr exch sub 5 1 roll 4 -1 roll 1.000000 ",
            "cvr exch sub 4 1 roll 3 -1 roll 1.000000 cvr exch sub 3 1 roll 2 -1 roll 1.000000 ",
            "cvr exch sub 2 1 roll pop }"
        );
        assert_eq!(run(program, &[0.25]), Some(vec![0.0, 0.0, 0.0, 0.25]));
    }

    #[test]
    fn malformed_or_runaway_programs_have_no_result() {
        assert!(Program::parse(b"1 2 add").is_none()); // no braces
        assert!(Program::parse(b"{ 1 2 add").is_none()); // unclosed
        assert!(Program::parse(b"{ 1 { 2 } }").is_none()); // a procedure no `if` takes
        assert!(Program::parse(b"{ nonsense }").is_none());
        assert_eq!(run("{ add }", &[]), None); // stack underflow
        assert_eq!(run("{ 1 0 idiv }", &[]), None);
        assert_eq!(run("{ -1 sqrt }", &[]), None);
        let long = format!("{{ {} }}", "1 pop ".repeat(MAX_STEPS));
        assert_eq!(run(&long, &[]), None);
    }
}
