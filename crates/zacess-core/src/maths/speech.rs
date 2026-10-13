//! Natural speech for equations, units and the symbols electronics uses.
//!
//! The aim is how a lecturer says it: `V_{out}` is "V out", `x^2` is "x squared", `kΩ` is
//! "kilo-ohms". Every symbol has a spoken name, so a screen reader never falls silent or spells
//! out a code point.

use super::{Node, parse};

/// Speaks an equation written as plain text or simple LaTeX.
pub fn speak(source: &str) -> String {
    speak_node(&parse(source))
}

/// Speaks a parsed equation.
pub fn speak_node(node: &Node) -> String {
    match node {
        Node::Number(n) => n.clone(),
        Node::Identifier(i) => identifier(i),
        Node::Operator(o) => operator(o).to_string(),
        Node::Text(t) => t.trim_start_matches('\\').to_string(),
        Node::Space => String::new(),
        Node::Row(items) => join(items.iter().map(speak_node)),
        Node::Sub { base, sub } => join([speak_node(base), speak_node(sub)]),
        Node::Sup { base, sup } => power(&speak_node(base), sup),
        Node::SubSup { base, sub, sup } => power(&join([speak_node(base), speak_node(sub)]), sup),
        Node::Frac {
            numerator,
            denominator,
        } => {
            let (n, d) = (speak_node(numerator), speak_node(denominator));
            if is_simple(numerator) && is_simple(denominator) {
                format!("{n} over {d}")
            } else {
                format!("the fraction {n} over {d} end fraction")
            }
        }
        Node::Sqrt(inner) => format!("the square root of {}", speak_node(inner)),
    }
}

fn join(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_simple(node: &Node) -> bool {
    match node {
        Node::Row(items) => items.len() <= 1 && items.iter().all(is_simple),
        Node::Number(_) | Node::Identifier(_) => true,
        _ => false,
    }
}

fn power(base: &str, exponent: &Node) -> String {
    let plain = match exponent {
        Node::Row(items) if items.len() == 1 => &items[0],
        other => other,
    };
    match plain {
        Node::Number(n) if n == "2" => format!("{base} squared"),
        Node::Number(n) if n == "3" => format!("{base} cubed"),
        _ => format!("{base} to the power {}", speak_node(exponent)),
    }
}

fn identifier(text: &str) -> String {
    match text {
        "sin" => "sine".into(),
        "cos" => "cos".into(),
        "tan" => "tan".into(),
        "ln" => "natural log".into(),
        "log" => "log".into(),
        "exp" => "exp".into(),
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => symbol(c)
                    .map(str::to_string)
                    .unwrap_or_else(|| c.to_string()),
                _ => other.to_string(),
            }
        }
    }
}

fn operator(text: &str) -> &str {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => symbol(c).unwrap_or(text),
        _ => text,
    }
}

/// The spoken name of a maths or engineering symbol.
pub fn symbol(c: char) -> Option<&'static str> {
    Some(match c {
        '=' => "equals",
        '+' => "plus",
        '-' | '\u{2212}' => "minus",
        '\u{d7}' | '\u{22c5}' | '*' => "times",
        '/' | '\u{f7}' => "over",
        '\u{b1}' => "plus or minus",
        '<' => "less than",
        '>' => "greater than",
        '\u{2264}' => "less than or equal to",
        '\u{2265}' => "greater than or equal to",
        '\u{2260}' => "not equal to",
        '\u{2248}' => "approximately",
        '\u{221d}' => "proportional to",
        '\u{2192}' => "to",
        '(' => "open bracket",
        ')' => "close bracket",
        '[' => "open square bracket",
        ']' => "close square bracket",
        '|' => "bar",
        '%' => "percent",
        '\u{b0}' => "degrees",
        '\u{2032}' => "prime",
        '\u{221e}' => "infinity",
        '\u{2211}' => "sum of",
        '\u{222b}' => "integral of",
        '\u{221a}' => "square root of",
        '\u{2220}' => "angle",
        '\u{2225}' => "in parallel with",
        '\u{394}' | '\u{2206}' => "delta",
        '\u{2207}' => "nabla",
        '\u{3a9}' => "ohm",
        '\u{b5}' | '\u{3bc}' => "micro",
        '\u{3b1}' => "alpha",
        '\u{3b2}' => "beta",
        '\u{3b3}' => "gamma",
        '\u{3b4}' => "delta",
        '\u{3b5}' => "epsilon",
        '\u{3b8}' => "theta",
        '\u{3bb}' => "lambda",
        '\u{3c0}' => "pi",
        '\u{3c1}' => "rho",
        '\u{3c3}' => "sigma",
        '\u{3c4}' => "tau",
        '\u{3c6}' => "phi",
        '\u{3c9}' => "omega",
        _ => return None,
    })
}

const PREFIXES: &[(char, &str)] = &[
    ('T', "tera"),
    ('G', "giga"),
    ('M', "mega"),
    ('k', "kilo"),
    ('m', "milli"),
    ('\u{b5}', "micro"),
    ('\u{3bc}', "micro"),
    ('u', "micro"),
    ('n', "nano"),
    ('p', "pico"),
];

const UNITS: &[(&str, &str)] = &[
    ("\u{3a9}", "ohms"),
    ("V", "volts"),
    ("A", "amps"),
    ("W", "watts"),
    ("Hz", "hertz"),
    ("F", "farads"),
    ("H", "henries"),
    ("s", "seconds"),
    ("m", "metres"),
    ("g", "grams"),
    ("J", "joules"),
    ("N", "newtons"),
    ("Pa", "pascals"),
    ("dB", "decibels"),
    ("rad", "radians"),
    ("K", "kelvin"),
    ("C", "coulombs"),
    ("T", "tesla"),
    ("Wb", "webers"),
];

/// Speaks a unit such as `kΩ`, `mA` or `Hz`. Returns `None` for text that is not a unit.
pub fn speak_unit(token: &str) -> Option<String> {
    let token = token.trim_end_matches(['.', ',', ';', ':']);
    if let Some((_, name)) = UNITS.iter().find(|(u, _)| *u == token) {
        return Some((*name).to_string());
    }
    let mut chars = token.chars();
    let first = chars.next()?;
    let rest: String = chars.collect();
    let (_, prefix) = PREFIXES.iter().find(|(p, _)| *p == first)?;
    let (_, name) = UNITS.iter().find(|(u, _)| *u == rest)?;
    Some(format!("{prefix}-{name}"))
}

/// Speaks a value with its unit, such as `4.7 kΩ` as "4.7 kilo-ohms".
pub fn speak_quantity(value: &str, unit: &str) -> String {
    match speak_unit(unit) {
        Some(spoken) => format!("{value} {spoken}"),
        None => format!("{value} {unit}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscripts_read_naturally_and_powers_as_a_lecturer_says_them() {
        assert_eq!(speak("V_{out} = I R"), "V out equals I R");
        assert_eq!(speak("P = I^2 R"), "P equals I squared R");
        assert_eq!(speak("V^3"), "V cubed");
        assert_eq!(speak("e^{j \\omega t}"), "e to the power j omega t");
        assert_eq!(speak("x_1^2"), "x 1 squared");
    }

    #[test]
    fn fractions_roots_and_functions() {
        assert_eq!(speak("\\frac{1}{2}"), "1 over 2");
        assert_eq!(
            speak("\\frac{R_2}{R_1 + R_2}"),
            "the fraction R 2 over R 1 plus R 2 end fraction"
        );
        assert_eq!(speak("\\sqrt{2} \\pi"), "the square root of 2 pi");
        assert_eq!(
            speak("sin(\\theta) \\approx \\theta"),
            "sine open bracket theta close bracket approximately theta"
        );
    }

    #[test]
    fn engineering_symbols_and_units_have_names() {
        assert_eq!(
            speak("Z = R \\angle 45\u{b0}"),
            "Z equals R angle 45 degrees"
        );
        assert_eq!(speak("R_1 \\parallel R_2"), "R 1 in parallel with R 2");
        assert_eq!(speak("12 \u{b1} 5%"), "12 plus or minus 5 percent");
        assert_eq!(speak_unit("k\u{3a9}").as_deref(), Some("kilo-ohms"));
        assert_eq!(speak_unit("mA").as_deref(), Some("milli-amps"));
        assert_eq!(speak_unit("\u{b5}F").as_deref(), Some("micro-farads"));
        assert_eq!(speak_unit("Hz").as_deref(), Some("hertz"));
        assert_eq!(speak_unit("hello"), None);
        assert_eq!(speak_quantity("4.7", "k\u{3a9}"), "4.7 kilo-ohms");
        assert_eq!(speak_quantity("3", "apples"), "3 apples");
    }
}
