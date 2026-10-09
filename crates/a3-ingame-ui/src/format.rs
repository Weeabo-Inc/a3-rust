//! The C `printf` formatting the engine applies to its HUD texts: stringtable formats such as
//! `STR_UI_AMMO` (`"%d | %d"`) or `STR_UI_SPEED_FREEFALL` (`"SPD %.0fkmph"`) and literal ones
//! (`"| %d"`, `"x%d"`).

/// One argument of [`sprintf`].
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Int(i64),
    Float(f64),
    Str(String),
}

impl From<i32> for Arg {
    fn from(v: i32) -> Self {
        Arg::Int(i64::from(v))
    }
}

impl From<u32> for Arg {
    fn from(v: u32) -> Self {
        Arg::Int(i64::from(v))
    }
}

impl From<f64> for Arg {
    fn from(v: f64) -> Self {
        Arg::Float(v)
    }
}

impl From<&str> for Arg {
    fn from(v: &str) -> Self {
        Arg::Str(v.to_owned())
    }
}

/// Formats `format` like C's `sprintf`: `%d %i %u %x %X %c %s %f %e %g %%` with flags
/// (`-+ 0#`), width and precision. An integer conversion of a float argument (and the reverse)
/// converts the value, where C would misread it; missing arguments format as zero/empty.
pub fn sprintf(format: &str, args: &[Arg]) -> String {
    let mut out = String::with_capacity(format.len() + 8);
    let mut args = args.iter();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }
        let mut left = false;
        let mut plus = false;
        let mut space = false;
        let mut zero = false;
        while let Some(&f) = chars.peek() {
            match f {
                '-' => left = true,
                '+' => plus = true,
                ' ' => space = true,
                '0' => zero = true,
                '#' => {}
                _ => break,
            }
            chars.next();
        }
        let mut width = 0usize;
        while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
            width = width * 10 + d as usize;
            chars.next();
        }
        let mut precision = None;
        if chars.peek() == Some(&'.') {
            chars.next();
            let mut p = 0usize;
            while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                p = p * 10 + d as usize;
                chars.next();
            }
            precision = Some(p);
        }
        // Length modifiers carry no meaning here.
        while matches!(chars.peek(), Some('l' | 'h' | 'L' | 'I' | 'q' | 'z')) {
            chars.next();
        }
        let Some(conversion) = chars.next() else {
            out.push('%');
            break;
        };
        let arg = args.next();
        let int = || match arg {
            Some(Arg::Int(i)) => *i,
            Some(Arg::Float(f)) => *f as i64,
            _ => 0,
        };
        let float = || match arg {
            Some(Arg::Int(i)) => *i as f64,
            Some(Arg::Float(f)) => *f,
            _ => 0.0,
        };
        let (body, numeric) = match conversion {
            'd' | 'i' => (int().to_string(), true),
            'u' => ((int() as u64).to_string(), true),
            'x' => (format!("{:x}", int()), true),
            'X' => (format!("{:X}", int()), true),
            'c' => (
                char::from_u32(int() as u32)
                    .map(String::from)
                    .unwrap_or_default(),
                false,
            ),
            's' => {
                let s = match arg {
                    Some(Arg::Str(s)) => s.clone(),
                    Some(Arg::Int(i)) => i.to_string(),
                    Some(Arg::Float(f)) => f.to_string(),
                    None => String::new(),
                };
                let s = match precision {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                };
                (s, false)
            }
            'f' | 'F' => (format!("{:.*}", precision.unwrap_or(6), float()), true),
            'e' | 'E' => {
                let s = c_exponent(float(), precision.unwrap_or(6));
                (
                    if conversion == 'E' {
                        s.to_uppercase()
                    } else {
                        s
                    },
                    true,
                )
            }
            'g' | 'G' => (format!("{}", float()), true),
            other => {
                out.push('%');
                out.push(other);
                continue;
            }
        };
        let body = if numeric && !body.starts_with('-') {
            if plus {
                format!("+{body}")
            } else if space {
                format!(" {body}")
            } else {
                body
            }
        } else {
            body
        };
        let len = body.chars().count();
        if len >= width {
            out.push_str(&body);
        } else if left {
            out.push_str(&body);
            out.extend(std::iter::repeat_n(' ', width - len));
        } else if zero && numeric {
            let (sign, digits) = match body.chars().next() {
                Some(s @ ('-' | '+' | ' ')) => (Some(s), &body[1..]),
                _ => (None, &body[..]),
            };
            out.extend(sign);
            out.extend(std::iter::repeat_n('0', width - len));
            out.push_str(digits);
        } else {
            out.extend(std::iter::repeat_n(' ', width - len));
            out.push_str(&body);
        }
    }
    out
}

/// `%e`: `d.ddde+XX` with at least two exponent digits.
fn c_exponent(v: f64, precision: usize) -> String {
    let s = format!("{v:.precision$e}");
    match s.split_once('e') {
        Some((mantissa, exp)) => {
            let (sign, digits) = match exp.strip_prefix('-') {
                Some(d) => ('-', d),
                None => ('+', exp),
            };
            format!("{mantissa}e{sign}{digits:0>2}")
        }
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hud_formats() {
        assert_eq!(sprintf("%d | %d", &[30.into(), 9.into()]), "30 | 9");
        assert_eq!(sprintf("| %d", &[9.into()]), "| 9");
        assert_eq!(sprintf("x%d", &[2.into()]), "x2");
        assert_eq!(sprintf("SPD %.0fkmph", &[201.0.into()]), "SPD 201kmph");
        assert_eq!(sprintf("ALT %.0fm", &[1499.6.into()]), "ALT 1500m");
    }

    #[test]
    fn flags_width_and_precision() {
        assert_eq!(sprintf("%03d", &[7.into()]), "007");
        assert_eq!(sprintf("%-4d|", &[7.into()]), "7   |");
        assert_eq!(sprintf("%+d", &[7.into()]), "+7");
        assert_eq!(sprintf("%05.1f", &[(-2.26).into()]), "-02.3");
        assert_eq!(sprintf("%s-%s", &["a".into(), "b".into()]), "a-b");
        assert_eq!(sprintf("100%%", &[]), "100%");
        assert_eq!(sprintf("%.2e", &[1234.5.into()]), "1.23e+03");
        assert_eq!(sprintf("%d", &[]), "0", "a missing argument is zero");
    }
}
