//! Byte-safe tag scanning shared by HTML and single-file component extractors.

pub(super) struct MarkupTag<'a> {
    pub(super) name: &'a str,
    pub(super) closing: bool,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) terminated: bool,
}

/// Parses one markup tag and skips `>` characters inside quoted attributes.
pub(super) fn markup_tag(text: &str, start: usize) -> Option<MarkupTag<'_>> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'<') {
        return None;
    }
    let mut name_start = start + 1;
    let closing = bytes.get(name_start) == Some(&b'/');
    if closing {
        name_start += 1;
    }
    if !bytes.get(name_start)?.is_ascii_alphabetic() {
        return None;
    }
    let mut name_end = name_start + 1;
    while bytes
        .get(name_end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        name_end += 1;
    }
    if bytes
        .get(name_end)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
    {
        return None;
    }
    let (end, terminated) = markup_end(bytes, start);
    Some(MarkupTag {
        name: &text[name_start..name_end],
        closing,
        start,
        end,
        terminated,
    })
}

pub(super) fn markup_end(bytes: &[u8], start: usize) -> (usize, bool) {
    let mut quote = None;
    for (index, &byte) in bytes.iter().enumerate().skip(start + 1) {
        match quote {
            Some(delimiter) if byte == delimiter => quote = None,
            Some(_) => {}
            None => match byte {
                b'\'' | b'"' => quote = Some(byte),
                b'>' => return (index + 1, true),
                _ => {}
            },
        }
    }
    (bytes.len(), false)
}

pub(super) fn raw_element_close<'a>(
    text: &'a str,
    lowercase: &str,
    from: usize,
    name: &str,
) -> Option<MarkupTag<'a>> {
    let needle = if name.eq_ignore_ascii_case("script") {
        "</script"
    } else if name.eq_ignore_ascii_case("style") {
        "</style"
    } else if name.eq_ignore_ascii_case("title") {
        "</title"
    } else if name.eq_ignore_ascii_case("textarea") {
        "</textarea"
    } else if name.eq_ignore_ascii_case("iframe") {
        "</iframe"
    } else if name.eq_ignore_ascii_case("noembed") {
        "</noembed"
    } else if name.eq_ignore_ascii_case("noframes") {
        "</noframes"
    } else if name.eq_ignore_ascii_case("noscript") {
        "</noscript"
    } else if name.eq_ignore_ascii_case("xmp") {
        "</xmp"
    } else {
        return None;
    };
    let mut search_from = from;
    while let Some(start) = lowercase[search_from..]
        .find(needle)
        .map(|offset| search_from + offset)
    {
        if let Some(tag) = markup_tag(text, start)
            && tag.closing
            && tag.name.eq_ignore_ascii_case(name)
        {
            return Some(tag);
        }
        search_from = start + 2;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::raw_element_close;

    #[test]
    fn raw_elements_close_only_at_their_own_closing_tag() {
        for name in [
            "script", "style", "title", "textarea", "iframe", "noembed", "noframes", "noscript",
            "xmp",
        ] {
            let text = format!("<{name}>a </{name}x> </div> b</{name} >tail");
            let lowercase = text.to_ascii_lowercase();
            let tag = raw_element_close(&text, &lowercase, 0, name);
            assert!(
                tag.is_some_and(|tag| tag.closing && tag.name.eq_ignore_ascii_case(name)),
                "{name}"
            );
            let unclosed = format!("<{name}>a");
            assert!(raw_element_close(&unclosed, &unclosed, 0, name).is_none());
        }
        assert!(raw_element_close("<div></div>", "<div></div>", 0, "div").is_none());
    }
}
