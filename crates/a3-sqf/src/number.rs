//! Number formatting and parsing as the engine's C runtime does it.
//!
//! SQF numbers are `f32`. The engine formats them with the static UCRT
//! `printf` family on the float widened to `double`, and parses them with
//! `atof`. Both run with the SSE flush-to-zero and denormals-are-zero modes
//! on, so subnormal floats read and print as zero. See
//! `docs/re/sqf-semantics.md` ("Numbers").

/// Flushes a subnormal float to zero, keeping its sign (the SSE FTZ/DAZ
/// modes the engine runs with).
pub fn ftz(x: f32) -> f32 {
    if x.is_subnormal() {
        0.0f32.copysign(x)
    } else {
        x
    }
}

/// `printf("%g", (double)x)`: six significant digits, the shorter of the
/// fixed and exponent forms, trailing zeros removed, a two-digit exponent
/// (`1e+06`). Infinities print `inf`/`-inf`, NaN `nan`, `-nan` or
/// `-nan(ind)` (the default NaN of the FPU).
pub fn format_g(x: f32) -> String {
    let x = ftz(x);
    if !x.is_finite() {
        return non_finite(x);
    }
    let v = f64::from(x);
    if v == 0.0 {
        return if v.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    const PRECISION: i32 = 6;
    let sci = format!("{:.*e}", (PRECISION - 1) as usize, v);
    let (mantissa, exp) = sci.split_once('e').expect("exponent format");
    let exp: i32 = exp.parse().expect("exponent digits");
    if !(-4..PRECISION).contains(&exp) {
        let mantissa = strip_fraction_zeros(mantissa);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exp.unsigned_abs())
    } else {
        let decimals = (PRECISION - 1 - exp).max(0) as usize;
        strip_fraction_zeros(&format!("{v:.decimals$}")).to_string()
    }
}

/// `printf("%0.<digits>f", (double)x)`: the exact decimal value rounded to
/// `digits` places, ties to even.
pub fn format_fixed(x: f32, digits: usize) -> String {
    let x = ftz(x);
    if !x.is_finite() {
        return non_finite(x);
    }
    format!("{:.*}", digits, f64::from(x))
}

/// The UCRT spelling of an infinity or NaN.
fn non_finite(x: f32) -> String {
    let neg = x.is_sign_negative();
    if x.is_infinite() {
        return if neg { "-inf" } else { "inf" }.to_string();
    }
    // Widening to double quiets the NaN and keeps the payload. The FPU's
    // default NaN (sign set, quiet bit only) is the "indeterminate" one.
    let payload = x.to_bits() & 0x003F_FFFF;
    match (neg, payload == 0) {
        (true, true) => "-nan(ind)".to_string(),
        (true, false) => "-nan".to_string(),
        (false, _) => "nan".to_string(),
    }
}

fn strip_fraction_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// `(float)atof(s)`: the longest prefix `strtod` accepts, else 0. Leading
/// whitespace, a sign, then a decimal number with optional exponent, a
/// hexadecimal number (`0x1F`, `0x1.8p3`), `inf`/`infinity` or
/// `nan`/`nan(chars)`, all case-insensitive.
pub fn parse_prefix(s: &str) -> f32 {
    ftz(strtod(s) as f32)
}

fn strtod(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let rest = &b[i..];
    let value = if starts_with_ci(rest, b"inf") {
        Some(f64::INFINITY)
    } else if starts_with_ci(rest, b"nan") {
        Some(f64::NAN)
    } else if rest.len() > 2
        && rest[0] == b'0'
        && (rest[1] | 0x20) == b'x'
        && (rest[2].is_ascii_hexdigit()
            || (rest[2] == b'.' && rest.get(3).is_some_and(u8::is_ascii_hexdigit)))
    {
        Some(parse_hex(&rest[2..]))
    } else {
        parse_decimal(rest)
    };
    // Nothing converted: +0, whatever the sign.
    match value {
        Some(v) if neg => -v,
        Some(v) => v,
        None => 0.0,
    }
}

fn starts_with_ci(b: &[u8], prefix: &[u8]) -> bool {
    b.len() >= prefix.len() && b[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn parse_decimal(b: &[u8]) -> Option<f64> {
    let mut i = 0;
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    let mut end = i;
    if i < b.len() && (b[i] | 0x20) == b'e' {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            end = j;
        }
    }
    let text = std::str::from_utf8(&b[..end]).unwrap_or("0");
    // Rust needs a digit before the point.
    let text = if text.starts_with('.') {
        format!("0{text}")
    } else {
        text.to_string()
    };
    text.parse().ok()
}

fn parse_hex(b: &[u8]) -> f64 {
    let mut mantissa = 0f64;
    let mut exp = 0i32;
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_hexdigit() {
        mantissa = mantissa * 16.0 + f64::from(hex(b[i]));
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_hexdigit() {
            mantissa = mantissa * 16.0 + f64::from(hex(b[i]));
            exp -= 4;
            i += 1;
        }
    }
    if i < b.len() && (b[i] | 0x20) == b'p' {
        let mut j = i + 1;
        let mut eneg = false;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            eneg = b[j] == b'-';
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            let mut e = 0i32;
            while j < b.len() && b[j].is_ascii_digit() {
                e = e.saturating_mul(10).saturating_add(i32::from(b[j] - b'0'));
                j += 1;
            }
            exp = exp.saturating_add(if eneg { -e } else { e });
        }
    }
    mantissa * 2f64.powi(exp)
}

fn hex(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        _ => (c | 0x20) - b'a' + 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_format_matches_the_oracle() {
        let cases: &[(f32, &str)] = &[
            (7.0, "7"),
            (0.1, "0.1"),
            (-2.5, "-2.5"),
            (123456.0, "123456"),
            (1234567.0, "1.23457e+06"),
            (1e6, "1e+06"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.0 / 3.0, "0.333333"),
            (1e38, "1e+38"),
            (16777217.0, "1.67772e+07"),
            (999999.5, "1e+06"),
            (-0.0, "-0"),
            (1.5e-7, "1.5e-07"),
            (f32::INFINITY, "inf"),
            (f32::NEG_INFINITY, "-inf"),
            (1e-39, "0"),
            (-1e-39, "-0"),
        ];
        for (x, want) in cases {
            assert_eq!(format_g(*x), *want, "{x:e}");
        }
    }

    #[test]
    fn nan_spellings() {
        let default_nan = (-1.0f32).sqrt();
        assert_eq!(format_g(default_nan), "-nan(ind)");
        assert_eq!(format_g(-default_nan), "nan");
        assert_eq!(format_g(f32::from_bits(0xFFC0_0001)), "-nan");
    }

    #[test]
    fn fixed_rounds_ties_to_even() {
        assert_eq!(format_fixed(0.5, 0), "0");
        assert_eq!(format_fixed(1.5, 0), "2");
        assert_eq!(format_fixed(2.5, 0), "2");
        assert_eq!(format_fixed(-0.5, 0), "-0");
        assert_eq!(format_fixed(0.125, 2), "0.12");
        assert_eq!(format_fixed(1.005, 2), "1.00");
        assert_eq!(format_fixed(123.456, 20), "123.45600128173828125000");
        assert_eq!(format_fixed(1e30, 2), "1000000015047466219876688855040.00");
        assert_eq!(format_fixed(f32::INFINITY, 2), "inf");
    }

    #[test]
    fn atof_prefixes() {
        let cases: &[(&str, f32)] = &[
            ("inf", f32::INFINITY),
            ("-1e40", f32::NEG_INFINITY),
            ("1.#INF", 1.0),
            ("  12abc", 12.0),
            ("0x10", 16.0),
            ("1e", 1.0),
            (".5", 0.5),
            ("-.5e1", -5.0),
            ("", 0.0),
            ("abc", 0.0),
            ("1e-40", 0.0),
            ("0x1.8p1", 3.0),
            ("INFINITY", f32::INFINITY),
            ("+7", 7.0),
            ("-", 0.0),
            (".", 0.0),
        ];
        for (s, want) in cases {
            assert_eq!(parse_prefix(s).to_bits(), want.to_bits(), "{s:?}");
        }
        assert!(parse_prefix("nan").is_nan());
        assert!(!parse_prefix("nan").is_sign_negative());
    }
}
