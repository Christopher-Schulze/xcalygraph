//! ECMAScript numeric literal values and property-key spelling.

pub(super) fn property_key(literal: &str) -> Option<String> {
    let (radix, digits) = if let Some(digits) = literal
        .strip_prefix("0x")
        .or_else(|| literal.strip_prefix("0X"))
    {
        (16, digits)
    } else if let Some(digits) = literal
        .strip_prefix("0b")
        .or_else(|| literal.strip_prefix("0B"))
    {
        (2, digits)
    } else if let Some(digits) = literal
        .strip_prefix("0o")
        .or_else(|| literal.strip_prefix("0O"))
    {
        (8, digits)
    } else {
        (10, literal)
    };
    let clean = remove_separators(digits, radix)?;
    let value = if radix == 10 {
        decimal_value(&clean)?
    } else {
        radix_value(&clean, radix)?
    };
    format_number(value)
}

fn remove_separators(text: &str, radix: u32) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'_' {
            let valid = |byte: &u8| (*byte as char).is_digit(radix);
            if !index
                .checked_sub(1)
                .and_then(|i| bytes.get(i))
                .is_some_and(valid)
                || !bytes.get(index + 1).is_some_and(valid)
            {
                return None;
            }
        }
    }
    Some(text.chars().filter(|ch| *ch != '_').collect())
}

fn decimal_value(text: &str) -> Option<f64> {
    if !text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        || text.starts_with(['+', '-'])
        || text.as_bytes().first() == Some(&b'0')
            && text.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
    {
        return None;
    }
    let value = text.parse::<f64>().ok()?;
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let fraction = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let digits: String = mantissa.chars().filter(|ch| *ch != '.').collect();
    let significant = digits.trim_start_matches('0').trim_end_matches('0');
    if significant.len() <= 20 {
        return Some(value);
    }
    // RoundMVResult permits either 20-digit endpoint. Omit results on which they disagree.
    let prefix = significant.get(..20)?.parse::<u128>().ok()?;
    let exponent = exponent.parse::<i64>().ok()?;
    let removed = i64::try_from(digits.trim_start_matches('0').len().checked_sub(20)?).ok()?;
    let power = exponent
        .checked_sub(i64::try_from(fraction).ok()?)?
        .checked_add(removed)?;
    let low = format!("{prefix}e{power}").parse::<f64>().ok()?;
    let high = format!("{}e{power}", prefix + 1).parse::<f64>().ok()?;
    (low == high).then_some(value)
}

fn radix_value(text: &str, radix: u32) -> Option<f64> {
    let width = match radix {
        2 => 1,
        8 => 3,
        16 => 4,
        _ => return None,
    };
    let mut bits = 0usize;
    let mut prefix = 0u64;
    let mut sticky = false;
    for ch in text.chars() {
        let digit = ch.to_digit(radix)?;
        for shift in (0..width).rev() {
            let bit = (digit >> shift) & 1;
            if bits == 0 && bit == 0 {
                continue;
            }
            bits += 1;
            if bits <= 54 {
                prefix = (prefix << 1) | u64::from(bit);
            } else {
                sticky |= bit != 0;
            }
        }
    }
    if bits <= 53 {
        return Some(prefix as f64);
    }
    if bits > 1024 {
        return Some(f64::INFINITY);
    }
    let mut high = prefix >> 1;
    if prefix & 1 != 0 && (sticky || high & 1 != 0) {
        high += 1;
    }
    Some((high as f64) * 2f64.powi(i32::try_from(bits - 53).ok()?))
}

fn format_number(value: f64) -> Option<String> {
    if value == 0.0 {
        return Some("0".to_owned());
    }
    if value.is_infinite() {
        return Some("Infinity".to_owned());
    }
    let shortest = value.to_string();
    let digits: String = shortest.chars().filter(|ch| *ch != '.').collect();
    let precision = digits
        .trim_start_matches('0')
        .trim_end_matches('0')
        .len()
        .checked_sub(1)?;
    let rounded = format!("{value:.precision$e}");
    let (mantissa, exponent) = rounded.split_once('e')?;
    let exponent = exponent.parse::<i32>().ok()?;
    let digits: String = mantissa.chars().filter(|ch| *ch != '.').collect();
    let digits = digits.trim_end_matches('0');
    if exponent >= 21 || exponent <= -7 {
        let (first, rest) = digits.split_at(1);
        let decimal = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let sign = if exponent >= 0 { "+" } else { "" };
        return Some(format!("{first}{decimal}e{sign}{exponent}"));
    }
    let decimal = exponent + 1;
    if decimal <= 0 {
        return Some(format!("0.{}{digits}", "0".repeat((-decimal) as usize)));
    }
    let decimal = decimal as usize;
    if decimal >= digits.len() {
        return Some(format!("{digits}{}", "0".repeat(decimal - digits.len())));
    }
    Some(format!("{}.{}", &digits[..decimal], &digits[decimal..]))
}

#[cfg(test)]
mod tests {
    use super::property_key;

    #[test]
    fn rejects_invalid_and_implementation_dependent_numbers() {
        for text in ["", "01", "-1", "0x", "0x_1", "1__0", "1e_3", "1n", "NaN"] {
            assert_eq!(property_key(text), None, "{text}");
        }
        // The endpoints allowed by RoundMVResult straddle the midpoint between 1 and its successor.
        assert_eq!(
            property_key("1.00000000000000011102230246251565404236316680908203125"),
            None
        );
        assert_eq!(
            property_key("1.2345678901234567890123").as_deref(),
            Some("1.2345678901234567")
        );
    }

    #[test]
    fn rounds_radix_integers_once_at_ieee_boundaries() {
        for (literal, expected) in [
            ("0x20000000000001", "9007199254740992"),
            ("0x20000000000003", "9007199254740996"),
            ("0x149ea49c2894c29", "92862869855095860"),
            ("0b1_010", "10"),
            ("0o77", "63"),
        ] {
            assert_eq!(
                property_key(literal).as_deref(),
                Some(expected),
                "{literal}"
            );
        }
        assert_eq!(
            property_key(&format!("0x{}", "f".repeat(256))).as_deref(),
            Some("Infinity")
        );
    }
}
