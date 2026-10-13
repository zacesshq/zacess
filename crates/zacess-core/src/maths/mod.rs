//! Equations as structure: parsed from plain text or simple LaTeX into a tree that can be rendered
//! as MathML, spoken naturally and walked piece by piece from the keyboard.
//!
//! The parser covers what lecture material mostly needs: identifiers, numbers, operators,
//! subscripts, superscripts, fractions, roots, Greek letters, common functions and the symbols
//! electronics uses. Anything it does not understand is kept as text, so nothing is lost.

pub mod speech;

use crate::export::escape;

/// One piece of an equation.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Number(String),
    Identifier(String),
    Operator(String),
    Text(String),
    Space,
    Row(Vec<Node>),
    Sub {
        base: Box<Node>,
        sub: Box<Node>,
    },
    Sup {
        base: Box<Node>,
        sup: Box<Node>,
    },
    SubSup {
        base: Box<Node>,
        sub: Box<Node>,
        sup: Box<Node>,
    },
    Frac {
        numerator: Box<Node>,
        denominator: Box<Node>,
    },
    Sqrt(Box<Node>),
}

impl Node {
    /// The parts a reader can step into, in reading order.
    pub fn children(&self) -> Vec<&Node> {
        match self {
            Node::Row(items) => items.iter().collect(),
            Node::Sub { base, sub } => vec![base, sub],
            Node::Sup { base, sup } => vec![base, sup],
            Node::SubSup { base, sub, sup } => vec![base, sub, sup],
            Node::Frac {
                numerator,
                denominator,
            } => vec![numerator, denominator],
            Node::Sqrt(inner) => vec![inner],
            _ => Vec::new(),
        }
    }

    /// What kind of part this is, for announcing where the reader is.
    pub fn role(&self) -> &'static str {
        match self {
            Node::Number(_) => "number",
            Node::Identifier(_) => "identifier",
            Node::Operator(_) => "operator",
            Node::Text(_) => "text",
            Node::Space => "space",
            Node::Row(_) => "expression",
            Node::Sub { .. } => "subscript",
            Node::Sup { .. } => "superscript",
            Node::SubSup { .. } => "subscript and superscript",
            Node::Frac { .. } => "fraction",
            Node::Sqrt(_) => "square root",
        }
    }

    /// The node at a path of child indexes from this one.
    pub fn at(&self, path: &[usize]) -> Option<&Node> {
        let mut node = self;
        for &i in path {
            node = *node.children().get(i)?;
        }
        Some(node)
    }
}

/// Parses an equation. The result is always a [`Node::Row`].
pub fn parse(source: &str) -> Node {
    let tokens = tokenise(source);
    let mut pos = 0;
    Node::Row(parse_sequence(&tokens, &mut pos, None))
}

/// The inner MathML of an equation, without the `<math>` element.
pub fn to_mathml(node: &Node) -> String {
    match node {
        Node::Number(n) => format!("<mn>{}</mn>", escape(n)),
        Node::Identifier(i) => format!("<mi>{}</mi>", escape(i)),
        Node::Operator(o) => format!("<mo>{}</mo>", escape(o)),
        Node::Text(t) => format!("<mtext>{}</mtext>", escape(t)),
        Node::Space => "<mspace width=\"0.2em\"/>".to_string(),
        Node::Row(items) => format!(
            "<mrow>{}</mrow>",
            items.iter().map(to_mathml).collect::<String>()
        ),
        Node::Sub { base, sub } => format!("<msub>{}{}</msub>", to_mathml(base), to_mathml(sub)),
        Node::Sup { base, sup } => format!("<msup>{}{}</msup>", to_mathml(base), to_mathml(sup)),
        Node::SubSup { base, sub, sup } => {
            format!(
                "<msubsup>{}{}{}</msubsup>",
                to_mathml(base),
                to_mathml(sub),
                to_mathml(sup)
            )
        }
        Node::Frac {
            numerator,
            denominator,
        } => {
            format!(
                "<mfrac>{}{}</mfrac>",
                to_mathml(numerator),
                to_mathml(denominator)
            )
        }
        Node::Sqrt(inner) => format!("<msqrt>{}</msqrt>", to_mathml(inner)),
    }
}

/// Steps through an equation one part at a time, the way arrow keys would.
///
/// `move_next` and `move_previous` move between siblings, `enter` steps into a part with inner
/// structure and `leave` steps back out. Each move reports what is now under the cursor.
#[derive(Debug, Clone)]
pub struct Navigator<'a> {
    root: &'a Node,
    path: Vec<usize>,
}

/// Where the navigator is and what it hears there.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub role: &'static str,
    pub speech: String,
    pub mathml: String,
    /// One-based index among the siblings and how many siblings there are.
    pub index: usize,
    pub of: usize,
    pub depth: usize,
}

impl<'a> Navigator<'a> {
    /// Starts on the first part of the equation.
    pub fn new(root: &'a Node) -> Self {
        let path = if root.children().is_empty() {
            Vec::new()
        } else {
            vec![0]
        };
        Navigator { root, path }
    }

    pub fn current(&self) -> &'a Node {
        self.root.at(&self.path).unwrap_or(self.root)
    }

    fn siblings(&self) -> Vec<&'a Node> {
        match self.path.split_last() {
            Some((_, parent)) => self.root.at(parent).map(Node::children).unwrap_or_default(),
            None => vec![self.root],
        }
    }

    pub fn position(&self) -> Position {
        let node = self.current();
        let index = self.path.last().map_or(0, |i| i + 1);
        Position {
            role: node.role(),
            speech: speech::speak_node(node),
            mathml: to_mathml(node),
            index: index.max(1),
            of: self.siblings().len().max(1),
            depth: self.path.len(),
        }
    }

    /// Moves to the next sibling. Stays put at the end and says so through the position.
    pub fn move_next(&mut self) -> Position {
        let count = self.siblings().len();
        if let Some(last) = self.path.last_mut()
            && *last + 1 < count
        {
            *last += 1;
        }
        self.position()
    }

    pub fn move_previous(&mut self) -> Position {
        if let Some(last) = self.path.last_mut()
            && *last > 0
        {
            *last -= 1;
        }
        self.position()
    }

    /// Steps into the current part, when it has parts of its own.
    pub fn enter(&mut self) -> Position {
        if !self.current().children().is_empty() {
            self.path.push(0);
        }
        self.position()
    }

    /// Steps out to the part that holds the current one.
    pub fn leave(&mut self) -> Position {
        if self.path.len() > 1 {
            self.path.pop();
        }
        self.position()
    }

    /// Reads the whole equation.
    pub fn whole(&self) -> String {
        speech::speak_node(self.root)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(String),
    Letter(char),
    Word(String),
    Command(String),
    Operator(String),
    Sup,
    Sub,
    Open,
    Close,
    Paren(char),
}

const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "log", "ln", "exp", "max", "min", "lim", "det", "sinh", "cosh", "tanh",
];

fn tokenise(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '\\' => {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && chars[end].is_ascii_alphabetic() {
                    end += 1;
                }
                if end == start && end < chars.len() {
                    end += 1;
                }
                out.push(Token::Command(chars[start..end].iter().collect()));
                i = end;
            }
            '^' => {
                out.push(Token::Sup);
                i += 1;
            }
            '_' => {
                out.push(Token::Sub);
                i += 1;
            }
            '{' => {
                out.push(Token::Open);
                i += 1;
            }
            '}' => {
                out.push(Token::Close);
                i += 1;
            }
            '(' | ')' | '[' | ']' | '|' => {
                out.push(Token::Paren(c));
                i += 1;
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                out.push(Token::Number(chars[start..i].iter().collect()));
            }
            c if c.is_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_alphabetic() {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if word.chars().count() == 1 {
                    out.push(Token::Letter(c));
                } else if FUNCTIONS.contains(&word.as_str()) {
                    out.push(Token::Word(word));
                } else {
                    // a run of letters is a product of variables unless it is a known function
                    out.extend(word.chars().map(Token::Letter));
                }
            }
            _ => {
                out.push(Token::Operator(c.to_string()));
                i += 1;
            }
        }
    }
    out
}

fn parse_sequence(tokens: &[Token], pos: &mut usize, until: Option<&Token>) -> Vec<Node> {
    let mut items: Vec<Node> = Vec::new();
    while *pos < tokens.len() {
        if until.is_some_and(|t| &tokens[*pos] == t) {
            *pos += 1;
            return items;
        }
        match &tokens[*pos] {
            Token::Sup | Token::Sub => {
                let is_sup = tokens[*pos] == Token::Sup;
                *pos += 1;
                let script = parse_atom(tokens, pos).unwrap_or(Node::Row(Vec::new()));
                let base = items.pop().unwrap_or(Node::Row(Vec::new()));
                items.push(attach_script(base, script, is_sup));
            }
            _ => {
                if let Some(node) = parse_atom(tokens, pos) {
                    items.push(node);
                }
            }
        }
    }
    items
}

/// Adds a script to a base, folding `x_1^2` into one node with both.
fn attach_script(base: Node, script: Node, is_sup: bool) -> Node {
    let script = name_from_letters(script);
    match (base, is_sup) {
        (Node::Sub { base, sub }, true) => Node::SubSup {
            base,
            sub,
            sup: Box::new(script),
        },
        (Node::Sup { base, sup }, false) => Node::SubSup {
            base,
            sub: Box::new(script),
            sup,
        },
        (base, true) => Node::Sup {
            base: Box::new(base),
            sup: Box::new(script),
        },
        (base, false) => Node::Sub {
            base: Box::new(base),
            sub: Box::new(script),
        },
    }
}

/// Plain letters grouped in a script spell a name such as `out`, not a product of variables.
/// Greek letters stay separate, so `j\omega t` is still read symbol by symbol.
fn name_from_letters(script: Node) -> Node {
    match &script {
        Node::Row(items)
            if items.len() > 1
                && items
                    .iter()
                    .all(|i| matches!(i, Node::Identifier(x) if x.len() == 1 && x.is_ascii())) =>
        {
            Node::Identifier(
                items
                    .iter()
                    .map(|i| match i {
                        Node::Identifier(x) => x.as_str(),
                        _ => "",
                    })
                    .collect(),
            )
        }
        _ => script,
    }
}

fn parse_atom(tokens: &[Token], pos: &mut usize) -> Option<Node> {
    let token = tokens.get(*pos)?;
    *pos += 1;
    Some(match token {
        Token::Number(n) => Node::Number(n.clone()),
        Token::Letter(c) => Node::Identifier(c.to_string()),
        Token::Word(w) => Node::Identifier(w.clone()),
        Token::Operator(o) => Node::Operator(match o.as_str() {
            "-" => "\u{2212}".to_string(),
            "*" => "\u{22c5}".to_string(),
            other => other.to_string(),
        }),
        Token::Open => Node::Row(parse_sequence(tokens, pos, Some(&Token::Close))),
        Token::Close | Token::Sup | Token::Sub => return None,
        Token::Paren(c) => Node::Operator(c.to_string()),
        Token::Command(name) => command(name, tokens, pos),
    })
}

fn command(name: &str, tokens: &[Token], pos: &mut usize) -> Node {
    let ident = |s: &str| Node::Identifier(s.to_string());
    let op = |s: &str| Node::Operator(s.to_string());
    match name {
        "frac" => {
            let numerator = parse_atom(tokens, pos).unwrap_or(Node::Row(Vec::new()));
            let denominator = parse_atom(tokens, pos).unwrap_or(Node::Row(Vec::new()));
            Node::Frac {
                numerator: Box::new(numerator),
                denominator: Box::new(denominator),
            }
        }
        "sqrt" => Node::Sqrt(Box::new(
            parse_atom(tokens, pos).unwrap_or(Node::Row(Vec::new())),
        )),
        "alpha" => ident("\u{3b1}"),
        "beta" => ident("\u{3b2}"),
        "gamma" => ident("\u{3b3}"),
        "delta" => ident("\u{3b4}"),
        "epsilon" => ident("\u{3b5}"),
        "theta" => ident("\u{3b8}"),
        "lambda" => ident("\u{3bb}"),
        "mu" => ident("\u{3bc}"),
        "pi" => ident("\u{3c0}"),
        "sigma" => ident("\u{3c3}"),
        "tau" => ident("\u{3c4}"),
        "phi" => ident("\u{3c6}"),
        "omega" => ident("\u{3c9}"),
        "Delta" => ident("\u{394}"),
        "Omega" => ident("\u{3a9}"),
        "Sigma" | "sum" => op("\u{2211}"),
        "int" => op("\u{222b}"),
        "infty" => ident("\u{221e}"),
        "times" => op("\u{d7}"),
        "cdot" => op("\u{22c5}"),
        "pm" => op("\u{b1}"),
        "leq" | "le" => op("\u{2264}"),
        "geq" | "ge" => op("\u{2265}"),
        "neq" | "ne" => op("\u{2260}"),
        "approx" => op("\u{2248}"),
        "rightarrow" | "to" => op("\u{2192}"),
        "angle" => op("\u{2220}"),
        "parallel" => op("\u{2225}"),
        "left" | "right" => Node::Row(Vec::new()),
        "," | ";" | " " => Node::Space,
        other => Node::Text(format!("\\{other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_identifiers_numbers_and_operators() {
        assert_eq!(
            to_mathml(&parse("V = IR")),
            "<mrow><mi>V</mi><mo>=</mo><mi>I</mi><mi>R</mi></mrow>"
        );
        assert_eq!(
            to_mathml(&parse("2.5 - x")),
            "<mrow><mn>2.5</mn><mo>\u{2212}</mo><mi>x</mi></mrow>"
        );
        assert_eq!(
            to_mathml(&parse("sin(x)")),
            "<mrow><mi>sin</mi><mo>(</mo><mi>x</mi><mo>)</mo></mrow>"
        );
    }

    #[test]
    fn parses_scripts_fractions_and_roots() {
        assert_eq!(
            to_mathml(&parse("x^2")),
            "<mrow><msup><mi>x</mi><mn>2</mn></msup></mrow>"
        );
        assert_eq!(
            to_mathml(&parse("V_{out}")),
            "<mrow><msub><mi>V</mi><mi>out</mi></msub></mrow>"
        );
        assert_eq!(
            to_mathml(&parse("x_1^2")),
            "<mrow><msubsup><mi>x</mi><mn>1</mn><mn>2</mn></msubsup></mrow>"
        );
        assert_eq!(
            to_mathml(&parse("x^2_1")),
            "<mrow><msubsup><mi>x</mi><mn>1</mn><mn>2</mn></msubsup></mrow>"
        );
        assert_eq!(
            to_mathml(&parse("\\frac{1}{2}")),
            "<mrow><mfrac><mrow><mn>1</mn></mrow><mrow><mn>2</mn></mrow></mfrac></mrow>"
        );
        assert_eq!(
            to_mathml(&parse("\\sqrt{2}\\pi")),
            "<mrow><msqrt><mrow><mn>2</mn></mrow></msqrt><mi>\u{3c0}</mi></mrow>"
        );
    }

    #[test]
    fn keeps_what_it_does_not_understand() {
        assert_eq!(
            to_mathml(&parse("\\unknown")),
            "<mrow><mtext>\\unknown</mtext></mrow>"
        );
        assert_eq!(
            to_mathml(&parse("a < b")),
            "<mrow><mi>a</mi><mo>&lt;</mo><mi>b</mi></mrow>"
        );
        assert_eq!(to_mathml(&parse("")), "<mrow></mrow>");
        assert_eq!(
            to_mathml(&parse("x^")),
            "<mrow><msup><mi>x</mi><mrow></mrow></msup></mrow>"
        );
    }

    #[test]
    fn the_navigator_walks_siblings_and_steps_into_structure() {
        let node = parse("V_{out} = \\frac{R_2}{R_1 + R_2} V_{in}");
        let mut nav = Navigator::new(&node);
        let first = nav.position();
        assert_eq!(
            (first.role, first.index, first.of, first.depth),
            ("subscript", 1, 4, 1)
        );
        assert_eq!(first.speech, "V out");
        assert_eq!(nav.move_next().speech, "equals");
        let fraction = nav.move_next();
        assert_eq!(fraction.role, "fraction");
        assert_eq!(
            fraction.speech,
            "the fraction R 2 over R 1 plus R 2 end fraction"
        );
        assert_eq!(nav.enter().speech, "R 2");
        assert_eq!(nav.move_next().speech, "R 1 plus R 2");
        assert_eq!(
            nav.move_next().speech,
            "R 1 plus R 2",
            "stays on the last sibling"
        );
        assert_eq!(nav.leave().role, "fraction");
        assert_eq!(nav.move_next().speech, "V in");
        assert_eq!(nav.move_previous().role, "fraction");
        assert_eq!(
            nav.whole(),
            "V out equals the fraction R 2 over R 1 plus R 2 end fraction V in"
        );
    }
}
