//! Exact arithmetic for the AI, which is unreliable at sums. No shell, network or app access.
use anyhow::{bail, Result};

/// Works out `expression` ("10*40 + 100", "18% * 2340") and says it back with the result.
pub(super) fn run(expression: &str) -> Result<String> {
    let value = evaluate(expression)?;
    // Rounded to cents, without a ".0" that would be read out as "point zero".
    let value = (value * 100.0).round() / 100.0;
    let shown = if value.fract() == 0.0 && value.abs() < 9e15 { format!("{}", value as i64) } else { format!("{value}") };
    Ok(format!("{} = {shown}", expression.trim()))
}

/// Evaluates + - * / with brackets, unary minus and a postfix % (18% = 0.18). Nothing else.
fn evaluate(expression: &str) -> Result<f64> {
    struct P<'a> {
        s: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn skip(&mut self) {
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace() || *c == b',') {
                self.i += 1;
            }
        }
        fn peek(&mut self) -> Option<u8> {
            self.skip();
            self.s.get(self.i).copied()
        }
        fn sum(&mut self) -> Result<f64> {
            let mut v = self.product()?;
            while let Some(op @ (b'+' | b'-')) = self.peek() {
                self.i += 1;
                let r = self.product()?;
                v = if op == b'+' { v + r } else { v - r };
            }
            Ok(v)
        }
        fn product(&mut self) -> Result<f64> {
            let mut v = self.unary()?;
            while let Some(op @ (b'*' | b'/' | b'x' | b'X')) = self.peek() {
                self.i += 1;
                let r = self.unary()?;
                if op == b'/' {
                    if r == 0.0 {
                        bail!("Division by zero");
                    }
                    v /= r;
                } else {
                    v *= r;
                }
            }
            Ok(v)
        }
        fn unary(&mut self) -> Result<f64> {
            match self.peek() {
                Some(b'-') => {
                    self.i += 1;
                    Ok(-self.unary()?)
                }
                Some(b'+') => {
                    self.i += 1;
                    self.unary()
                }
                _ => self.percent(),
            }
        }
        fn percent(&mut self) -> Result<f64> {
            let mut v = self.atom()?;
            while self.peek() == Some(b'%') {
                self.i += 1;
                v /= 100.0;
            }
            Ok(v)
        }
        fn atom(&mut self) -> Result<f64> {
            if self.peek() == Some(b'(') {
                self.i += 1;
                let v = self.sum()?;
                if self.peek() != Some(b')') {
                    bail!("Unbalanced brackets");
                }
                self.i += 1;
                return Ok(v);
            }
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
                self.i += 1;
            }
            let digits = std::str::from_utf8(&self.s[start..self.i]).unwrap_or_default();
            digits.parse().map_err(|_| anyhow::anyhow!("Expected a number at “{}”", String::from_utf8_lossy(&self.s[start..])))
        }
    }
    let cleaned = expression.replace(['×', '·'], "*").replace('÷', "/").replace(['₹', '$', '€', '£'], "");
    let mut p = P { s: cleaned.as_bytes(), i: 0 };
    let v = p.sum()?;
    if p.peek().is_some() {
        bail!("Only numbers, + - * / ( ) and % can be used");
    }
    if !v.is_finite() {
        bail!("The result is too large");
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums() {
        assert_eq!(run("10*40 + 100 + 100").unwrap(), "10*40 + 100 + 100 = 600");
        assert_eq!(run("18% * 2340").unwrap(), "18% * 2340 = 421.2");
        assert!(run("2 / 0").is_err());
        assert!(run("rm -rf").is_err());
    }
}
