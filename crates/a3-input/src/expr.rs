//! Integer expressions used as key codes in `CfgDefaultKeysPresets`, e.g. `"256+0x25"`,
//! `"(0x00100000 +4)"`, `"0x00010000  + 128 + 1"`.
//!
//! RV evaluates these strings as expressions when it reads the preset _(the full grammar it
//! accepts is unknown; every shipped string uses integers, `+`, `*` and parentheses)_. This
//! evaluator accepts decimal and hex integers, `+ - *`, unary minus and parentheses.

use thiserror::Error;

/// A key-code expression that does not evaluate.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid key code expression {expression:?}: {reason}")]
pub struct KeyExprError {
    pub expression: String,
    pub reason: &'static str,
}

/// Evaluate a key-code expression to an integer.
pub fn eval_key_expression(expression: &str) -> Result<i64, KeyExprError> {
    let fail = |reason| KeyExprError {
        expression: expression.to_owned(),
        reason,
    };
    let tokens = tokenize(expression).map_err(fail)?;
    let mut parser = Parser { tokens, pos: 0 };
    let value = parser.sum().map_err(fail)?;
    if parser.pos != parser.tokens.len() {
        return Err(fail("unexpected trailing input"));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Number(i64),
    Plus,
    Minus,
    Star,
    Open,
    Close,
}

fn tokenize(src: &str) -> Result<Vec<Token>, &'static str> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let single = match bytes[i] {
            b'+' => Some(Token::Plus),
            b'-' => Some(Token::Minus),
            b'*' => Some(Token::Star),
            b'(' => Some(Token::Open),
            b')' => Some(Token::Close),
            _ => None,
        };
        if let Some(token) = single {
            tokens.push(token);
            i += 1;
            continue;
        }
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b'0'..=b'9' => {
                let (value, len) = if bytes[i..].starts_with(b"0x") || bytes[i..].starts_with(b"0X")
                {
                    let digits = bytes[i + 2..]
                        .iter()
                        .take_while(|b| b.is_ascii_hexdigit())
                        .count();
                    if digits == 0 {
                        return Err("hex prefix without digits");
                    }
                    let text = &src[i + 2..i + 2 + digits];
                    (
                        i64::from_str_radix(text, 16).map_err(|_| "number too large")?,
                        digits + 2,
                    )
                } else {
                    let digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
                    (
                        src[i..i + digits].parse().map_err(|_| "number too large")?,
                        digits,
                    )
                };
                tokens.push(Token::Number(value));
                i += len;
            }
            _ => return Err("unexpected character"),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.peek();
        self.pos += 1;
        t
    }

    fn sum(&mut self) -> Result<i64, &'static str> {
        let mut value = self.product()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.pos += 1;
                    value = value.checked_add(self.product()?).ok_or("overflow")?;
                }
                Some(Token::Minus) => {
                    self.pos += 1;
                    value = value.checked_sub(self.product()?).ok_or("overflow")?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn product(&mut self) -> Result<i64, &'static str> {
        let mut value = self.unary()?;
        while self.peek() == Some(Token::Star) {
            self.pos += 1;
            value = value.checked_mul(self.unary()?).ok_or("overflow")?;
        }
        Ok(value)
    }

    fn unary(&mut self) -> Result<i64, &'static str> {
        match self.next() {
            Some(Token::Minus) => self.unary()?.checked_neg().ok_or("overflow"),
            Some(Token::Number(n)) => Ok(n),
            Some(Token::Open) => {
                let value = self.sum()?;
                match self.next() {
                    Some(Token::Close) => Ok(value),
                    _ => Err("missing closing parenthesis"),
                }
            }
            _ => Err("expected a number"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_preset_expressions() {
        assert_eq!(eval_key_expression("256+0x25"), Ok(256 + 0x25));
        assert_eq!(eval_key_expression("(0x00100000 +4)"), Ok(0x0010_0004));
        assert_eq!(
            eval_key_expression("0x00010000  + 128 + 1"),
            Ok(0x0001_0081)
        );
        assert_eq!(eval_key_expression("0x00030000 +8+1"), Ok(0x0003_0009));
    }

    #[test]
    fn precedence_and_unary_minus() {
        assert_eq!(eval_key_expression("2 + 3 * 4"), Ok(14));
        assert_eq!(eval_key_expression("(2 + 3) * 4"), Ok(20));
        assert_eq!(eval_key_expression("-1 + 10 - 2"), Ok(7));
        assert_eq!(eval_key_expression("17"), Ok(17));
    }

    #[test]
    fn malformed_expressions_are_errors() {
        for bad in ["", "1+", "(1", "1)", "0x", "abc", "1 2", "DIK_W"] {
            assert!(eval_key_expression(bad).is_err(), "{bad:?}");
        }
    }
}
