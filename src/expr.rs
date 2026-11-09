//! Tiny arithmetic evaluator so price cells can hold spreadsheet-style
//! expressions such as `12.49-2.5` or `=3*1.99`.

pub fn eval(input: &str) -> Option<f64> {
    let s = input.trim().trim_start_matches('=').replace(['$', ','], "");
    if s.trim().is_empty() {
        return None;
    }
    let mut p = Parser { b: s.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    (p.i == p.b.len() && v.is_finite()).then_some(v)
}

/// Same as [`eval`] but treats empty/invalid input as zero.
pub fn eval_or_zero(input: &str) -> f64 {
    eval(input).unwrap_or(0.0)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.b.get(self.i).copied()
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(c @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term()?;
            v = if c == b'+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.factor()?;
        while let Some(c @ (b'*' | b'/' | b'x')) = self.peek() {
            self.i += 1;
            let r = self.factor()?;
            v = if c == b'/' { v / r } else { v * r };
        }
        Some(v)
    }
    fn factor(&mut self) -> Option<f64> {
        match self.peek()? {
            b'-' => {
                self.i += 1;
                Some(-self.factor()?)
            }
            b'+' => {
                self.i += 1;
                self.factor()
            }
            b'(' => {
                self.i += 1;
                let v = self.expr()?;
                (self.peek()? == b')').then(|| self.i += 1)?;
                Some(v)
            }
            _ => self.number(),
        }
    }
    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        while self.i < self.b.len() && (self.b[self.i].is_ascii_digit() || self.b[self.i] == b'.') {
            self.i += 1;
        }
        std::str::from_utf8(&self.b[start..self.i]).ok()?.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::eval;

    #[test]
    fn evaluates() {
        assert_eq!(eval("12.49-2.5"), Some(9.99));
        assert_eq!(eval("=13.99"), Some(13.99));
        assert_eq!(eval("$1,000.50"), Some(1000.5));
        assert_eq!(eval("2*(3+1)"), Some(8.0));
        assert_eq!(eval("2 x 3.5"), Some(7.0));
        assert_eq!(eval(""), None);
        assert_eq!(eval("abc"), None);
        assert_eq!(eval("1+"), None);
    }
}
