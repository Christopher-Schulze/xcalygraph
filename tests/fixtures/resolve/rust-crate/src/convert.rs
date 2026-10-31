//! Calls inside macro arguments and paths into inline modules.

pub fn parse_count(text: &str) -> usize {
    inner::digits(text)
}

mod inner {
    pub(super) fn digits(text: &str) -> usize {
        text.len()
    }
}

#[cfg(test)]
mod tests {
    use super::parse_count;

    #[test]
    fn counts_digits() {
        assert_eq!(parse_count("12"), super::inner::digits("12"));
        println!("{}", inner_check("x").len());
    }

    fn inner_check(text: &str) -> String {
        text.to_owned()
    }
}
