//! IEEE 754 binary16 ("half") to `f32`, as used for BMTR translations.

/// Converts the bits of an IEEE 754 half-precision float to `f32` (exact for every value,
/// including subnormals, infinities and NaN).
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x3ff);
    let magnitude = match (exponent, mantissa) {
        (0, 0) => 0,
        // Subnormal: mantissa * 2^-24, exactly representable as an f32.
        (0, m) => {
            let value = m as f32 * 2f32.powi(-24);
            return if sign == 0 { value } else { -value };
        }
        (0x1f, m) => 0xff << 23 | m << 13,
        (e, m) => (e + 127 - 15) << 23 | m << 13,
    };
    f32::from_bits(sign | magnitude)
}

#[cfg(test)]
mod tests {
    use super::f16_to_f32;

    #[test]
    fn converts_known_values() {
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert!(f16_to_f32(0x8000).is_sign_negative());
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x3555), 0.333_251_95);
        assert_eq!(f16_to_f32(0x7bff), 65504.0);
        assert_eq!(f16_to_f32(0x0001), 5.960_464_5e-8);
        assert_eq!(f16_to_f32(0x8001), -5.960_464_5e-8);
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        assert!(f16_to_f32(0x7e00).is_nan());
    }
}
