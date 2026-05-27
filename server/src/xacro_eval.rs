use std::collections::HashSet;
use crate::document::XacroProperty;

/// Evaluate a xacro expression (the inside of `${...}`) to an f64.
/// Returns `None` if the expression is malformed, references an undefined
/// variable, or contains a non-numeric value.
///
/// Supports:
/// - Numbers (integer and decimal)
/// - Operators: `+`, `-`, `*`, `/`, `%`, unary `-`
/// - Parentheses
/// - Math functions: `sin`, `cos`, `tan`, `abs`, `sqrt`, `radians`, `degrees`
/// - Constant: `pi`
/// - Variable references (with recursive resolution and cycle detection)
pub fn eval(expr: &str, props: &[XacroProperty]) -> Option<f64> {
    let mut visited: HashSet<&str> = HashSet::new();
    eval_inner(expr, props, &mut visited)
}

fn eval_inner<'a>(
    expr: &'a str,
    props: &'a [XacroProperty],
    visited: &mut HashSet<&'a str>,
) -> Option<f64> {
    let toks = tokenize(expr)?;
    let (val, pos) = parse_expr(&toks, 0, props, visited)?;
    if pos == toks.len() { Some(val) } else { None }
}

#[derive(Debug, Clone)]
enum Tok<'a> {
    Num(f64),
    Ident(&'a str),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
}

fn tokenize(s: &str) -> Option<Vec<Tok<'_>>> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b.is_ascii_digit() || (b == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            // Optional exponent: e+10, E-5
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let n: f64 = s[start..i].parse().ok()?;
            tokens.push(Tok::Num(n));
            continue;
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push(Tok::Ident(&s[start..i]));
            continue;
        }
        let t = match b {
            b'+' => Tok::Plus,
            b'-' => Tok::Minus,
            b'*' => Tok::Star,
            b'/' => Tok::Slash,
            b'%' => Tok::Percent,
            b'(' => Tok::LParen,
            b')' => Tok::RParen,
            _ => return None,
        };
        tokens.push(t);
        i += 1;
    }
    Some(tokens)
}

fn parse_expr<'a>(
    toks: &[Tok<'a>],
    pos: usize,
    props: &'a [XacroProperty],
    visited: &mut HashSet<&'a str>,
) -> Option<(f64, usize)> {
    let (mut left, mut p) = parse_term(toks, pos, props, visited)?;
    while p < toks.len() {
        match toks[p] {
            Tok::Plus => {
                let (r, np) = parse_term(toks, p + 1, props, visited)?;
                left += r;
                p = np;
            }
            Tok::Minus => {
                let (r, np) = parse_term(toks, p + 1, props, visited)?;
                left -= r;
                p = np;
            }
            _ => break,
        }
    }
    Some((left, p))
}

fn parse_term<'a>(
    toks: &[Tok<'a>],
    pos: usize,
    props: &'a [XacroProperty],
    visited: &mut HashSet<&'a str>,
) -> Option<(f64, usize)> {
    let (mut left, mut p) = parse_unary(toks, pos, props, visited)?;
    while p < toks.len() {
        match toks[p] {
            Tok::Star => {
                let (r, np) = parse_unary(toks, p + 1, props, visited)?;
                left *= r;
                p = np;
            }
            Tok::Slash => {
                let (r, np) = parse_unary(toks, p + 1, props, visited)?;
                if r == 0.0 { return None; }
                left /= r;
                p = np;
            }
            Tok::Percent => {
                let (r, np) = parse_unary(toks, p + 1, props, visited)?;
                if r == 0.0 { return None; }
                left %= r;
                p = np;
            }
            _ => break,
        }
    }
    Some((left, p))
}

fn parse_unary<'a>(
    toks: &[Tok<'a>],
    pos: usize,
    props: &'a [XacroProperty],
    visited: &mut HashSet<&'a str>,
) -> Option<(f64, usize)> {
    if pos >= toks.len() { return None; }
    match toks[pos] {
        Tok::Minus => {
            let (v, np) = parse_unary(toks, pos + 1, props, visited)?;
            Some((-v, np))
        }
        Tok::Plus => parse_unary(toks, pos + 1, props, visited),
        _ => parse_primary(toks, pos, props, visited),
    }
}

fn parse_primary<'a>(
    toks: &[Tok<'a>],
    pos: usize,
    props: &'a [XacroProperty],
    visited: &mut HashSet<&'a str>,
) -> Option<(f64, usize)> {
    if pos >= toks.len() { return None; }
    match toks[pos] {
        Tok::Num(n) => Some((n, pos + 1)),
        Tok::LParen => {
            let (v, np) = parse_expr(toks, pos + 1, props, visited)?;
            if np >= toks.len() || !matches!(toks[np], Tok::RParen) { return None; }
            Some((v, np + 1))
        }
        Tok::Ident(name) => {
            // Constant: pi
            if name.eq_ignore_ascii_case("pi") {
                return Some((std::f64::consts::PI, pos + 1));
            }
            // Function call: ident '(' expr ')'
            if pos + 1 < toks.len() && matches!(toks[pos + 1], Tok::LParen) {
                let (arg, np) = parse_expr(toks, pos + 2, props, visited)?;
                if np >= toks.len() || !matches!(toks[np], Tok::RParen) { return None; }
                let v = match name {
                    "sin"     => arg.sin(),
                    "cos"     => arg.cos(),
                    "tan"     => arg.tan(),
                    "abs"     => arg.abs(),
                    "sqrt"    => { if arg < 0.0 { return None; } arg.sqrt() }
                    "radians" => arg.to_radians(),
                    "degrees" => arg.to_degrees(),
                    _ => return None,
                };
                return Some((v, np + 1));
            }
            // Variable lookup with cycle detection
            if visited.contains(name) { return None; }
            let prop = props.iter().find(|p| p.name == name)?;
            visited.insert(prop.name.as_str());
            let val = eval_property_value(&prop.value, props, visited);
            visited.remove(prop.name.as_str());
            val.map(|v| (v, pos + 1))
        }
        _ => None,
    }
}

/// Resolve a xacro property value to an f64.
/// Accepts: plain numbers, `${expr}` wrappers, or naked expressions.
fn eval_property_value<'a>(
    value: &'a str,
    props: &'a [XacroProperty],
    visited: &mut HashSet<&'a str>,
) -> Option<f64> {
    let v = value.trim();
    if let Some(rest) = v.strip_prefix("${") {
        if let Some(inner) = rest.strip_suffix('}') {
            return eval_inner(inner, props, visited);
        }
    }
    // Bare numeric value
    if let Ok(n) = v.parse::<f64>() {
        return Some(n);
    }
    // Naked expression (e.g. value="a + b")
    eval_inner(v, props, visited)
}

/// Format an f64 for display as an inlay hint — strip trailing zeros, cap precision.
pub fn format_value(v: f64) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.6}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{Position, Range};

    fn prop(name: &str, value: &str) -> XacroProperty {
        XacroProperty {
            name: name.into(),
            value: value.into(),
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        }
    }

    #[test]
    fn simple_number() {
        assert_eq!(eval("42", &[]), Some(42.0));
        assert_eq!(eval("3.14", &[]), Some(3.14));
        assert_eq!(eval("-5", &[]), Some(-5.0));
    }

    #[test]
    fn arithmetic() {
        assert_eq!(eval("2 + 3", &[]), Some(5.0));
        assert_eq!(eval("10 / 2", &[]), Some(5.0));
        assert_eq!(eval("2 * (3 + 4)", &[]), Some(14.0));
        assert_eq!(eval("10 % 3", &[]), Some(1.0));
    }

    #[test]
    fn variables() {
        let p = vec![prop("len", "0.5"), prop("w", "0.3")];
        assert_eq!(eval("len", &p), Some(0.5));
        assert_eq!(eval("len / 2", &p), Some(0.25));
        assert_eq!(eval("len * w", &p), Some(0.15));
        assert_eq!(eval("-len", &p), Some(-0.5));
    }

    #[test]
    fn nested_xacro_value() {
        let p = vec![
            prop("a", "2"),
            prop("b", "${a + 1}"),
            prop("c", "${b * 2}"),
        ];
        assert_eq!(eval("c", &p), Some(6.0));
    }

    #[test]
    fn pi_and_functions() {
        let result = eval("pi/2", &[]).unwrap();
        assert!((result - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        let result = eval("sin(0)", &[]).unwrap();
        assert!(result.abs() < 1e-10);
    }

    #[test]
    fn cycle_returns_none() {
        let p = vec![prop("a", "${b}"), prop("b", "${a}")];
        assert_eq!(eval("a", &p), None);
    }

    #[test]
    fn undefined_var_returns_none() {
        assert_eq!(eval("missing_var", &[]), None);
    }

    #[test]
    fn divide_by_zero_returns_none() {
        assert_eq!(eval("1/0", &[]), None);
    }
}
