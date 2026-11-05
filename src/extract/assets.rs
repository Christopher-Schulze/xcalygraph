//! Assets (`docs/spec.md` section 5): image dimensions, audio and video durations from container
//! headers (MP4/MOV family, WAV, AVI), SVG ids, text and links. The summary (`png 512x512`,
//! `mp4 1280x720 92.4s`) becomes the file doc, so `show` and `find` carry it.

use crate::domain::facts::{
    self, AssetMeta, Facts, NO_ARITY, Ref, RefKind, SymKind, Symbol, TextSegment,
};

/// SVG ids kept per file.
const SVG_IDS: usize = 200;
const SVG_TEXT_CHARS: usize = 600;

pub fn extract(path: &str, content: &[u8]) -> Facts {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    if extension == "svg" {
        return svg(&String::from_utf8_lossy(content));
    }
    let mut meta = AssetMeta {
        format: extension.clone(),
        ..AssetMeta::default()
    };
    match extension.as_str() {
        "mp4" | "mov" | "m4v" | "m4a" => mp4(content, &mut meta),
        "wav" => wav(content, &mut meta),
        "avi" => avi(content, &mut meta),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" => {
            if let Ok(size) = imagesize::blob_size(content) {
                meta.width = size.width as u32;
                meta.height = size.height as u32;
            }
        }
        _ => {}
    }
    Facts {
        doc: summary(&meta),
        asset: Some(meta),
        ..Facts::default()
    }
}

fn summary(meta: &AssetMeta) -> String {
    let mut summary = meta.format.clone();
    if meta.width > 0 && meta.height > 0 {
        summary.push_str(&format!(" {}x{}", meta.width, meta.height));
    }
    if meta.duration_ms > 0 {
        summary.push_str(&format!(" {:.1}s", f64::from(meta.duration_ms) / 1000.0));
    }
    summary
}

/// ISO base media (MP4, MOV, M4A, M4V): duration from `moov/mvhd`, size from the first
/// `trak/tkhd` with a non-zero width.
fn mp4(content: &[u8], meta: &mut AssetMeta) {
    let Some(moov) = child_box(content, b"moov") else {
        return;
    };
    if let Some(mvhd) = child_box(moov, b"mvhd") {
        let (timescale, duration) = match mvhd.first() {
            Some(1) => (be32(mvhd, 20), be64(mvhd, 24)),
            _ => (be32(mvhd, 12), be32(mvhd, 16).map(u64::from)),
        };
        if let (Some(timescale), Some(duration)) = (timescale, duration)
            && timescale > 0
        {
            meta.duration_ms = u32::try_from(duration.saturating_mul(1000) / u64::from(timescale))
                .unwrap_or(u32::MAX);
        }
    }
    let mut rest = moov;
    while let Some((kind, body, next)) = next_box(rest) {
        rest = next;
        if kind != b"trak" {
            continue;
        }
        let Some(tkhd) = child_box(body, b"tkhd") else {
            continue;
        };
        let offset = if tkhd.first() == Some(&1) { 88 } else { 76 };
        if let (Some(width), Some(height)) = (be32(tkhd, offset), be32(tkhd, offset + 4))
            && width >> 16 > 0
        {
            meta.width = width >> 16;
            meta.height = height >> 16;
            break;
        }
    }
}

/// The body of the first box of `kind` among the boxes of `data`.
fn child_box<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut rest = data;
    while let Some((found, body, next)) = next_box(rest) {
        if found == kind {
            return Some(body);
        }
        rest = next;
    }
    None
}

/// `(type, body, rest)` of the box at the start of `data`.
fn next_box(data: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let size = be32(data, 0)? as usize;
    let kind = data.get(4..8)?;
    let (header, size) = match size {
        1 => (16, usize::try_from(be64(data, 8)?).ok()?),
        0 => (8, data.len()),
        size => (8, size),
    };
    if size < header || size > data.len() {
        return None;
    }
    Some((kind, &data[header..size], &data[size..]))
}

/// WAV: duration = `data` size / byte rate of `fmt `.
fn wav(content: &[u8], meta: &mut AssetMeta) {
    if content.get(0..4) != Some(b"RIFF") || content.get(8..12) != Some(b"WAVE") {
        return;
    }
    let (mut byte_rate, mut data_size) = (None, None);
    let mut offset = 12;
    while let (Some(id), Some(size)) = (content.get(offset..offset + 4), le32(content, offset + 4))
    {
        match id {
            b"fmt " => byte_rate = le32(content, offset + 16),
            b"data" => data_size = Some(size),
            _ => {}
        }
        offset += 8 + size as usize + (size as usize & 1);
    }
    if let (Some(rate), Some(size)) = (byte_rate, data_size)
        && rate > 0
    {
        meta.duration_ms =
            u32::try_from(u64::from(size) * 1000 / u64::from(rate)).unwrap_or(u32::MAX);
    }
}

/// AVI: `avih` main header (microseconds per frame, total frames, width, height).
fn avi(content: &[u8], meta: &mut AssetMeta) {
    if content.get(0..4) != Some(b"RIFF") || content.get(8..12) != Some(b"AVI ") {
        return;
    }
    let Some(position) = content
        .windows(4)
        .take(4096)
        .position(|window| window == b"avih")
    else {
        return;
    };
    let header = position + 8;
    if let (Some(micros), Some(frames), Some(width), Some(height)) = (
        le32(content, header),
        le32(content, header + 16),
        le32(content, header + 32),
        le32(content, header + 36),
    ) {
        meta.duration_ms =
            u32::try_from(u64::from(micros) * u64::from(frames) / 1000).unwrap_or(u32::MAX);
        meta.width = width;
        meta.height = height;
    }
}

fn be32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn be64(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn le32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// SVG: `id` attributes as constants (sprite symbols, gradients), `<title>`, `<desc>` and
/// `<text>` content as text, relative `href`/`xlink:href` targets as path references; the
/// view box size and the title as file doc.
fn svg(text: &str) -> Facts {
    let mut facts = Facts::default();
    let mut meta = AssetMeta {
        format: "svg".to_owned(),
        ..AssetMeta::default()
    };
    let mut line = 1u32;
    let mut position = 0;
    let mut collected = String::new();
    let mut title = String::new();
    let mut capture: Option<&'static str> = None;
    while let Some(offset) = text[position..].find('<') {
        let start = position + offset;
        let chunk = &text[position..start];
        if let Some(element) = capture {
            if element == "title" && title.is_empty() {
                title = facts::normalize_space(chunk);
            }
            if collected.chars().count() < SVG_TEXT_CHARS {
                collected.push(' ');
                collected.push_str(chunk.trim());
            }
        }
        line += chunk.matches('\n').count() as u32;
        let end = text[start..]
            .find('>')
            .map_or(text.len(), |offset| start + offset + 1);
        let tag = &text[start..end];
        let tag_line = line;
        line += tag.matches('\n').count() as u32;
        position = end;
        if tag.starts_with("<!") || tag.starts_with("<?") {
            continue;
        }
        let closing = tag.starts_with("</");
        let name = tag
            .trim_start_matches(['<', '/'])
            .split(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
            .next()
            .unwrap_or_default();
        capture = match (name, closing) {
            ("title", false) => Some("title"),
            ("desc" | "text" | "tspan", false) => Some("text"),
            (_, true) => None,
            _ => capture,
        };
        if closing {
            continue;
        }
        if name == "svg"
            && meta.width == 0
            && let Some(view_box) = attribute(tag, "viewBox")
        {
            let numbers: Vec<f64> = view_box
                .split([' ', ','])
                .filter_map(|number| number.parse().ok())
                .collect();
            if let [_, _, width, height] = numbers.as_slice() {
                meta.width = *width as u32;
                meta.height = *height as u32;
            }
        }
        if let Some(id) = attribute(tag, "id")
            && facts.symbols.len() < SVG_IDS
            && !id.is_empty()
        {
            facts.symbols.push(Symbol {
                kind: Some(SymKind::Const),
                name: id.to_owned().into(),
                line: tag_line,
                end_line: tag_line,
                signature: facts::clip_space(tag, 160).into(),
                exported: true,
                ..Symbol::default()
            });
        }
        for key in ["href", "xlink:href"] {
            if let Some(target) = attribute(tag, key) {
                let external = target.starts_with('#') || target.contains(':');
                if !external && !target.is_empty() {
                    facts.refs.push(Ref {
                        kind: RefKind::Path,
                        name: target.to_owned(),
                        qualifier: None,
                        scope: None,
                        line: tag_line,
                        arity: NO_ARITY,
                        byte: crate::domain::facts::NO_BYTE,
                    });
                }
            }
        }
    }
    let collected = facts::normalize_space(&collected);
    if !collected.is_empty() {
        facts.texts.push(TextSegment {
            heading: String::new(),
            locator: 1,
            text: collected,
        });
    }
    let summary = summary(&meta);
    facts.doc = if title.is_empty() {
        summary
    } else {
        facts::clip(&format!("{summary} {title}"), 400)
    };
    facts.asset = Some(meta);
    facts
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut search = 0;
    while let Some(found) = tag[search..].find(name) {
        let start = search + found;
        search = start + name.len();
        let boundary = start > 0 && tag.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = tag[search..].trim_start();
        if !boundary || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let quote = value.chars().next().filter(|ch| matches!(ch, '"' | '\''))?;
        let body = &value[1..];
        return body.find(quote).map(|end| &body[..end]);
    }
    None
}

/// Extensions of documents and assets a bare file name may reference; other literals need a
/// directory part to count as a path.
const REFERENCED_EXTENSIONS: [&str; 22] = [
    "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "mp4", "mov", "webm", "mkv", "avi", "m4v",
    "mp3", "wav", "m4a", "ogg", "pdf", "docx", "xlsx", "html", "md",
];

/// Literal file paths in code, templates and config (`"assets/logo.png"`, `url(img/bg.svg)`)
/// as path references of the innermost enclosing symbol; the resolver keeps those naming an
/// existing file of the root.
pub fn literal_paths(text: &str, facts: &mut Facts) {
    for (index, line) in text.lines().enumerate() {
        let number = index as u32 + 1;
        // Import and include specifiers are `imports` edges already.
        if facts.imports.iter().any(|import| import.line == number) {
            continue;
        }
        let mut path_literal = |literal: &str, computed: bool| {
            if !is_path_literal(literal) {
                return;
            }
            let scope = facts
                .symbols
                .iter()
                .rposition(|symbol| symbol.line <= number && number <= symbol.end_line)
                .map(|position| position as u32);
            let duplicate = facts.refs.iter().any(|reference| {
                reference.kind == RefKind::Path
                    && reference.scope == scope
                    && reference.name == literal
            });
            if !duplicate {
                facts.refs.push(Ref {
                    kind: RefKind::Path,
                    name: literal.to_owned(),
                    qualifier: computed.then(|| crate::domain::facts::COMPUTED_BASE.to_owned()),
                    scope,
                    line: number,
                    arity: NO_ARITY,
                    byte: crate::domain::facts::NO_BYTE,
                });
            }
        };
        // Quotes nested in template literals (`` `<img src="a.png">` ``) count too.
        for_each_quoted(line, |literal, before| {
            // `root / "src/main.rs"`, `dir.join("src/main.rs")` and a bare `"README.md"` hang
            // below a computed or unknown base: the same-named repository file is a guess.
            let before = before.trim_end();
            let literal = literal.trim();
            let computed =
                before.ends_with('/') || before.ends_with("join(") || !literal.contains('/');
            if is_path_literal(literal) {
                path_literal(literal, computed);
            } else {
                for_each_quoted(literal, |nested, _| path_literal(nested.trim(), computed));
            }
        });
    }
}

/// Contents of `"..."`, `'...'`, `` `...` `` and `url(...)` on one line.
fn for_each_quoted(line: &str, mut emit: impl FnMut(&str, &str)) {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if matches!(byte, b'"' | b'\'' | b'`') {
            if let Some(end) = line[index + 1..].find(byte as char) {
                emit(&line[index + 1..index + 1 + end], &line[..index]);
                index += end + 2;
                continue;
            }
            break;
        }
        // Byte comparison: `index` may sit inside a multi-byte character.
        if bytes[index..].starts_with(b"url(")
            && let Some(end) = line[index + 4..].find(')')
        {
            emit(
                line[index + 4..index + 4 + end].trim_matches(['"', '\'']),
                &line[..index],
            );
            index += end + 5;
            continue;
        }
        index += 1;
    }
}

fn is_path_literal(literal: &str) -> bool {
    let Some((stem, extension)) = literal.rsplit_once('.') else {
        return false;
    };
    let plain = literal
        .chars()
        .all(|ch| ch.is_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | '@' | '+'));
    (3..=200).contains(&literal.len())
        && plain
        && !stem.is_empty()
        && !stem.ends_with('/')
        && (1..=5).contains(&extension.len())
        && extension.chars().all(|ch| ch.is_ascii_alphanumeric())
        && extension.chars().any(|ch| ch.is_ascii_alphabetic())
        && (stem.contains('/')
            || REFERENCED_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn mp4_duration_and_track_size() {
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&92_400u32.to_be_bytes());
        let mut tkhd = vec![0u8; 84];
        tkhd[76..80].copy_from_slice(&(1280u32 << 16).to_be_bytes());
        tkhd[80..84].copy_from_slice(&(720u32 << 16).to_be_bytes());
        let trak = mp4_box(b"trak", &mp4_box(b"tkhd", &tkhd));
        let mut moov_body = mp4_box(b"mvhd", &mvhd);
        moov_body.extend(trak);
        let mut file = mp4_box(b"ftyp", b"isom0000");
        file.extend(mp4_box(b"moov", &moov_body));
        let facts = extract("media/intro.mp4", &file);
        assert_eq!(facts.doc, "mp4 1280x720 92.4s");
    }

    #[test]
    fn wav_avi_and_images() {
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend_from_slice(&16u32.to_le_bytes());
        let mut fmt = vec![0u8; 16];
        fmt[8..12].copy_from_slice(&176_400u32.to_le_bytes());
        wav.extend(fmt);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&352_800u32.to_le_bytes());
        assert_eq!(extract("a.wav", &wav).doc, "wav 2.0s");

        let mut avi = b"RIFF\0\0\0\0AVI LIST\0\0\0\0hdrlavih".to_vec();
        avi.extend_from_slice(&56u32.to_le_bytes());
        let mut header = vec![0u8; 56];
        header[0..4].copy_from_slice(&40_000u32.to_le_bytes());
        header[16..20].copy_from_slice(&250u32.to_le_bytes());
        header[32..36].copy_from_slice(&640u32.to_le_bytes());
        header[36..40].copy_from_slice(&480u32.to_le_bytes());
        avi.extend(header);
        assert_eq!(extract("b.avi", &avi).doc, "avi 640x480 10.0s");

        let gif = b"GIF89a\x10\x00\x20\x00\x00\x00\x00;";
        assert_eq!(extract("c.gif", gif).doc, "gif 16x32");
        assert_eq!(extract("broken.png", b"not a png").doc, "png");
        assert_eq!(extract("song.mp3", b"ID3").doc, "mp3");
    }

    #[test]
    fn literal_paths_in_code_and_css() {
        let mut facts = Facts::default();
        facts.symbols.push(Symbol {
            name: "render".to_owned().into(),
            line: 2,
            end_line: 4,
            ..Symbol::default()
        });
        literal_paths(
            "const LOGO = \"assets/logo.png\";\nfunction render() {\n  return `<img src=\"/img/a.svg\">` + load('config/app.yaml') + 'v1.2' + \"hello.world\";\n}\n.hero { background: url(../img/bg.webp); }\nfetch(\"https://x.io/a.png\")\n",
            &mut facts,
        );
        let refs: Vec<(String, u32, Option<u32>)> = facts
            .refs
            .iter()
            .map(|r| (r.name.clone(), r.line, r.scope))
            .collect();
        assert_eq!(
            refs,
            [
                ("assets/logo.png".to_owned(), 1, None),
                ("/img/a.svg".to_owned(), 3, Some(0)),
                ("config/app.yaml".to_owned(), 3, Some(0)),
                ("../img/bg.webp".to_owned(), 5, None),
            ]
        );
    }

    #[test]
    fn literal_paths_below_a_computed_base_are_marked() {
        let mut facts = Facts::default();
        literal_paths(
            "(project / \"src/main.rs\").write_text('x')\nroot.join(\"src/lib.rs\");\nopen('src/real.rs')\nopen('README.md')\n",
            &mut facts,
        );
        let names: Vec<_> = facts
            .refs
            .iter()
            .map(|reference| (reference.name.as_str(), reference.qualifier.is_some()))
            .collect();
        assert_eq!(
            names,
            [
                ("src/main.rs", true),
                ("src/lib.rs", true),
                ("src/real.rs", false),
                ("README.md", true)
            ]
        );
    }

    #[test]
    fn literal_paths_tolerate_multibyte_text() {
        let mut facts = Facts::default();
        literal_paths("Über ‘x’ «a» 'img/Ü.png' url(ä/b.svg)\n", &mut facts);
        let names: Vec<&str> = facts.refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["img/Ü.png", "ä/b.svg"]);
    }

    #[test]
    fn svg_ids_text_and_links() {
        let svg = "<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\n<title>Store icons</title>\n<symbol id=\"icon-store\"><path d=\"M0\"/></symbol>\n<use href=\"#icon-store\"/>\n<image xlink:href=\"img/bg.png\"/>\n<text x=\"1\">Open</text>\n</svg>\n";
        let facts = extract("assets/icons.svg", svg.as_bytes());
        let ids: Vec<(String, u32)> = facts
            .symbols
            .iter()
            .map(|symbol| (symbol.name.to_string(), symbol.line))
            .collect();
        assert_eq!(ids, [("icon-store".to_owned(), 4)]);
        let refs: Vec<(String, u32)> = facts
            .refs
            .iter()
            .map(|r| (r.name.clone(), r.line))
            .collect();
        assert_eq!(refs, [("img/bg.png".to_owned(), 6)]);
        assert_eq!(facts.doc, "svg 24x24 Store icons");
        assert_eq!(facts.texts[0].text, "Store icons Open");
    }
}
