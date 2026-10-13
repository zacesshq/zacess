//! Presentation MathML for the equations the importers recognise, from the shared equation parser
//! in [`crate::maths`].

/// Converts an equation to the inner markup of a `<math>` element.
pub fn to_mathml(source: &str) -> String {
    crate::maths::to_mathml(&crate::maths::parse(source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_numbers_and_operators() {
        assert_eq!(
            to_mathml("V = IR"),
            "<mrow><mi>V</mi><mo>=</mo><mi>I</mi><mi>R</mi></mrow>"
        );
        assert_eq!(
            to_mathml("2.5 - x"),
            "<mrow><mn>2.5</mn><mo>\u{2212}</mo><mi>x</mi></mrow>"
        );
        assert_eq!(
            to_mathml("sin(x)"),
            "<mrow><mi>sin</mi><mo>(</mo><mi>x</mi><mo>)</mo></mrow>"
        );
    }

    #[test]
    fn scripts_fractions_and_roots() {
        assert_eq!(
            to_mathml("x^2"),
            "<mrow><msup><mi>x</mi><mn>2</mn></msup></mrow>"
        );
        assert_eq!(
            to_mathml("V_{out}"),
            "<mrow><msub><mi>V</mi><mi>out</mi></msub></mrow>"
        );
        assert_eq!(
            to_mathml("x_1^2"),
            "<mrow><msubsup><mi>x</mi><mn>1</mn><mn>2</mn></msubsup></mrow>"
        );
        assert_eq!(
            to_mathml("\\frac{1}{2}"),
            "<mrow><mfrac><mrow><mn>1</mn></mrow><mrow><mn>2</mn></mrow></mfrac></mrow>"
        );
        assert_eq!(
            to_mathml("\\sqrt{2}\\pi"),
            "<mrow><msqrt><mrow><mn>2</mn></mrow></msqrt><mi>\u{3c0}</mi></mrow>"
        );
    }

    #[test]
    fn unknown_commands_and_markup_are_kept_safely() {
        assert_eq!(
            to_mathml("\\unknown"),
            "<mrow><mtext>\\unknown</mtext></mrow>"
        );
        assert_eq!(
            to_mathml("a < b"),
            "<mrow><mi>a</mi><mo>&lt;</mo><mi>b</mi></mrow>"
        );
        assert_eq!(to_mathml(""), "<mrow></mrow>");
        assert_eq!(
            to_mathml("x^"),
            "<mrow><msup><mi>x</mi><mrow></mrow></msup></mrow>"
        );
    }
}
