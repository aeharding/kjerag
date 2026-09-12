// WGSL permits implementation-defined division precision.  The selected CPU
// boundary requires correctly-rounded binary32. Estimate a normalized integer
// quotient with hardware division, then correct its EXACT integer remainder.
// The exponent, subnormals and RN-even rounding still use integer arithmetic.
fn div_f32_bits(a: u32, b: u32) -> u32 {
    let sign = (a ^ b) & 0x80000000u;
    let a_abs = a & 0x7fffffffu;
    let b_abs = b & 0x7fffffffu;
    let a_exp = a_abs >> 23u;
    let b_exp = b_abs >> 23u;
    let a_frac = a_abs & 0x007fffffu;
    let b_frac = b_abs & 0x007fffffu;

    if a_exp == 0xffu && a_frac != 0u { return a | 0x00400000u; }
    if b_exp == 0xffu && b_frac != 0u { return b | 0x00400000u; }
    if (a_abs == 0u && b_abs == 0u) || (a_exp == 0xffu && b_exp == 0xffu) {
        return 0xffc00000u;
    }
    if b_abs == 0u || a_exp == 0xffu { return sign | 0x7f800000u; }
    if a_abs == 0u || b_exp == 0xffu { return sign; }

    var ma = a_frac;
    var mb = b_frac;
    var ea = i32(a_exp) - 127;
    var eb = i32(b_exp) - 127;
    if a_exp == 0u {
        let top = 31u - countLeadingZeros(a_frac);
        ma = a_frac << (23u - top);
        ea = i32(top) - 149;
    } else {
        ma |= 0x00800000u;
    }
    if b_exp == 0u {
        let top = 31u - countLeadingZeros(b_frac);
        mb = b_frac << (23u - top);
        eb = i32(top) - 149;
    } else {
        mb |= 0x00800000u;
    }

    var remainder = ma;
    var quotient_exponent = ea - eb;
    if remainder < mb {
        remainder <<= 1u;
        quotient_exponent -= 1;
    }
    let numerator = remainder;
    var quotient = u32((f32(numerator) / f32(mb)) * 8388608.0);
    // WGSL bounds this normal division to 2.5 ULP. Both inputs convert
    // exactly, and multiplying by 2^23 is exact, so the quotient estimate
    // is within a few integer units. The true signed remainder is therefore
    // much smaller than 2^31: modular u32 arithmetic recovers it exactly
    // without a 48-bit product. See WGSL accuracy-of-concrete-expressions.
    var residue = bitcast<i32>((numerator << 23u) - quotient * mb);
    for (var correction = 0u; correction < 4u; correction++) {
        if residue < 0 {
            quotient -= 1u;
            residue += i32(mb);
        } else if u32(residue) >= mb {
            quotient += 1u;
            residue -= i32(mb);
        } else {
            break;
        }
    }
    if residue >= 0 && u32(residue) < mb {
        remainder = u32(residue);
    } else {
        // Retain the restoring reference when an estimate needs more than
        // four corrections. Conforming hardware does not take this path.
        quotient = 0u;
        remainder = numerator;
        for (var step = 0u; step < 24u; step++) {
            let bit = 23u - step;
            if remainder >= mb {
                remainder -= mb;
                quotient |= 1u << bit;
            }
            if step != 23u { remainder <<= 1u; }
        }
    }

    if quotient_exponent >= -126 {
        let twice_remainder = remainder << 1u;
        if twice_remainder > mb || (twice_remainder == mb && (quotient & 1u) != 0u) {
            quotient += 1u;
        }
        if quotient == 0x01000000u {
            quotient = 0x00800000u;
            quotient_exponent += 1;
        }
        if quotient_exponent > 127 { return sign | 0x7f800000u; }
        return sign | (u32(quotient_exponent + 127) << 23u) | (quotient & 0x007fffffu);
    }

    let shift = u32(-126 - quotient_exponent);
    if shift >= 25u { return sign; }
    var subnormal = quotient >> shift;
    let mask = (1u << shift) - 1u;
    let low = quotient & mask;
    let half = 1u << (shift - 1u);
    if low > half || (low == half && (remainder != 0u || (subnormal & 1u) != 0u)) {
        subnormal += 1u;
    }
    return sign | subnormal;
}

fn div_rn(a: f32, b: f32) -> f32 {
    return bitcast<f32>(div_f32_bits(bitcast<u32>(a), bitcast<u32>(b)));
}
