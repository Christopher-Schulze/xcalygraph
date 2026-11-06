//! Structural languages (`docs/spec.md` sections 3 and 5): symbols, includes and calls from line
//! patterns for Apex, Razor, Blade, EJS, Jinja, COBOL and XAML. Includes are path
//! references to the included file; calls resolve by unique name (section 6, inferred).

use crate::domain::name::Name;
use std::sync::OnceLock;

use regex::Regex;

use crate::domain::facts::{
    self, FLAG_ENTRY, FLAG_TEST, Facts, NO_ARITY, Ref, RefKind, SymKind, Symbol,
};
use crate::domain::filetype::FileType;

pub fn extract(file_type: FileType, path: &str, content: &[u8]) -> Facts {
    let text = String::from_utf8_lossy(content);
    let mut facts = match file_type {
        FileType::Apex => apex(&text),
        FileType::Razor => razor(path, &text),
        FileType::Blade => blade(&text),
        FileType::Ejs => ejs(&text),
        FileType::Jinja => jinja(&text),
        FileType::Cobol => cobol(&text),
        FileType::Xaml => xaml(path, &text),
        _ => Facts::default(),
    };
    facts.lines = text.lines().count() as u32;
    facts
}

fn regex(cell: &'static OnceLock<Option<Regex>>, pattern: &str) -> Option<&'static Regex> {
    cell.get_or_init(|| Regex::new(pattern).ok()).as_ref()
}

fn push_symbol(facts: &mut Facts, kind: SymKind, name: &str, line: u32, signature: &str) -> u32 {
    let parent = innermost(facts, line);
    facts.symbols.push(Symbol {
        kind: Some(kind),
        name: name.into(),
        parent,
        line,
        end_line: line,
        signature: facts::clip_space(signature, 160).into(),
        exported: true,
        ..Symbol::default()
    });
    facts.symbols.len() as u32 - 1
}

/// The innermost symbol whose line range contains `line`.
fn innermost(facts: &Facts, line: u32) -> Option<u32> {
    facts
        .symbols
        .iter()
        .rposition(|symbol| symbol.line <= line && line <= symbol.end_line)
        .map(|position| position as u32)
}

fn push_ref(facts: &mut Facts, kind: RefKind, name: &str, qualifier: Option<&str>, line: u32) {
    let scope = innermost(facts, line);
    let duplicate = facts.refs.iter().any(|reference| {
        reference.kind == kind
            && reference.scope == scope
            && reference.name == name
            && reference.qualifier.as_deref() == qualifier
    });
    if !duplicate && !name.is_empty() {
        facts.refs.push(Ref {
            kind,
            name: name.into(),
            qualifier: qualifier.map(Name::from),
            scope,
            line,
            arity: NO_ARITY,
            byte: crate::domain::facts::NO_BYTE,
        });
    }
}

/// Line (1-based) of a byte offset.
fn line_of(text: &str, offset: usize) -> u32 {
    text[..offset.min(text.len())].matches('\n').count() as u32 + 1
}

/// Brace-delimited end line of a declaration starting at `offset` (the first `{` after it).
fn block_end(text: &str, offset: usize) -> u32 {
    let mut depth = 0i32;
    let mut started = false;
    for (index, byte) in text.bytes().enumerate().skip(offset) {
        match byte {
            b'{' => {
                depth += 1;
                started = true;
            }
            b'}' => {
                depth -= 1;
                if started && depth <= 0 {
                    return line_of(text, index);
                }
            }
            b';' if !started => return line_of(text, index),
            _ => {}
        }
    }
    line_of(text, text.len())
}

const C_LIKE_KEYWORDS: [&str; 16] = [
    "if", "for", "while", "switch", "catch", "return", "new", "else", "do", "try", "when",
    "sizeof", "typeof", "nameof", "using", "lock",
];

/// Calls `f(` and `Type.f(` in C-like code between `start` and `end` lines.
fn c_like_calls(facts: &mut Facts, text: &str, skip_declarations: bool) {
    static CALL: OnceLock<Option<Regex>> = OnceLock::new();
    let Some(call) = regex(&CALL, r"(?:([A-Za-z_]\w*)\s*\.\s*)?\b([A-Za-z_]\w*)\s*\(") else {
        return;
    };
    for found in call.captures_iter(text) {
        let (Some(whole), Some(name)) = (found.get(0), found.get(2)) else {
            continue;
        };
        let line = line_of(text, whole.start());
        let name = name.as_str();
        let declared_here = skip_declarations
            && facts
                .symbols
                .iter()
                .any(|symbol| symbol.line == line && *symbol.name == *name);
        if C_LIKE_KEYWORDS.contains(&name.to_ascii_lowercase().as_str()) || declared_here {
            continue;
        }
        let qualifier = found.get(1).map(|qualifier| qualifier.as_str());
        push_ref(facts, RefKind::Call, name, qualifier, line);
    }
}

struct ApexDeclaration {
    offset: usize,
    kind: SymKind,
    name: String,
    test: bool,
    base: Option<String>,
    supertypes: Vec<String>,
}

/// Apex: classes, interfaces and enums with supertypes, methods, triggers (entrypoints),
/// `@isTest` classes and methods, calls.
fn apex(text: &str) -> Facts {
    static TYPE: OnceLock<Option<Regex>> = OnceLock::new();
    static METHOD: OnceLock<Option<Regex>> = OnceLock::new();
    static TRIGGER: OnceLock<Option<Regex>> = OnceLock::new();
    let mut facts = Facts::default();
    let (Some(type_decl), Some(method), Some(trigger)) = (
        regex(
            &TYPE,
            r"(?im)^\s*(@isTest\s+)?(?:(?:public|private|protected|global|virtual|abstract|with sharing|without sharing|inherited sharing|static)\s+)*(class|interface|enum)\s+(\w+)(?:\s+extends\s+(\w+))?(?:\s+implements\s+([\w\s,.]+?))?\s*\{",
        ),
        regex(
            &METHOD,
            r"(?im)^\s*(@isTest\s+)?(?:(?:public|private|protected|global|static|override|virtual|abstract|testmethod|webservice)\s+)+[\w<>,.\[\]\s]*?\b(\w+)\s*\([^;{]*\)\s*\{",
        ),
        regex(&TRIGGER, r"(?im)^\s*trigger\s+(\w+)\s+on\s+(\w+)"),
    ) else {
        return facts;
    };
    let mut declarations: Vec<ApexDeclaration> = Vec::new();
    for found in type_decl.captures_iter(text) {
        let (Some(whole), Some(keyword), Some(name)) = (found.get(0), found.get(2), found.get(3))
        else {
            continue;
        };
        let kind = match keyword.as_str().to_ascii_lowercase().as_str() {
            "interface" => SymKind::Iface,
            "enum" => SymKind::Enum,
            _ => SymKind::Class,
        };
        let supertypes = found
            .get(5)
            .map(|list| {
                list.as_str()
                    .split(',')
                    .map(|item| item.trim().to_owned())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        declarations.push(ApexDeclaration {
            offset: whole.start() + whole.as_str().len() - whole.as_str().trim_start().len(),
            kind,
            name: name.as_str().to_owned(),
            test: found.get(1).is_some(),
            base: found.get(4).map(|base| base.as_str().to_owned()),
            supertypes,
        });
    }
    for found in method.captures_iter(text) {
        let (Some(whole), Some(name)) = (found.get(0), found.get(2)) else {
            continue;
        };
        let name = name.as_str();
        if C_LIKE_KEYWORDS.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        let test =
            found.get(1).is_some() || whole.as_str().to_ascii_lowercase().contains("testmethod");
        declarations.push(ApexDeclaration {
            offset: whole.start() + whole.as_str().len() - whole.as_str().trim_start().len(),
            kind: SymKind::Method,
            name: name.to_owned(),
            test,
            base: None,
            supertypes: Vec::new(),
        });
    }
    declarations.sort_by_key(|declaration| declaration.offset);
    for ApexDeclaration {
        offset,
        kind,
        name,
        test,
        base,
        supertypes,
    } in declarations
    {
        let line = line_of(text, offset);
        let header = text[offset..].lines().next().unwrap_or_default();
        let id = push_symbol(&mut facts, kind, &name, line, header.trim_end_matches('{'));
        facts.symbols[id as usize].end_line = block_end(text, offset);
        let inherited_test = facts.symbols[id as usize]
            .parent
            .is_some_and(|parent| facts.symbols[parent as usize].flags & FLAG_TEST != 0);
        if test || inherited_test {
            facts.symbols[id as usize].flags |= FLAG_TEST;
        }
        if kind == SymKind::Method
            && let Some(parent) = facts.symbols[id as usize].parent
        {
            facts.symbols[id as usize].owner = facts.symbols[parent as usize].name;
        }
        if let Some(base) = base {
            push_ref(&mut facts, RefKind::Inherit, &base, None, line);
        }
        for supertype in supertypes {
            push_ref(&mut facts, RefKind::Implement, &supertype, None, line);
        }
    }
    for found in trigger.captures_iter(text) {
        let (Some(whole), Some(name)) = (found.get(0), found.get(1)) else {
            continue;
        };
        let line = line_of(text, whole.start());
        let id = push_symbol(
            &mut facts,
            SymKind::Fn,
            name.as_str(),
            line,
            whole.as_str().trim(),
        );
        facts.symbols[id as usize].end_line = block_end(text, whole.start());
        facts.symbols[id as usize].flags |= FLAG_ENTRY;
    }
    facts.symbols.sort_by_key(|symbol| symbol.line);
    reparent(&mut facts);
    c_like_calls(&mut facts, &strip_c_comments(text), true);
    facts
}

/// Parents by line containment after symbols were sorted.
fn reparent(facts: &mut Facts) {
    for index in 0..facts.symbols.len() {
        let (line, end) = (facts.symbols[index].line, facts.symbols[index].end_line);
        facts.symbols[index].parent = (0..index)
            .rev()
            .find(|&other| {
                let other = &facts.symbols[other];
                other.line <= line && end <= other.end_line && other.kind() != SymKind::Method
            })
            .map(|other| other as u32);
    }
}

/// Blanks `//` and `/* */` comments and string contents, keeping offsets and lines.
fn strip_c_comments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        match (bytes[index], bytes.get(index + 1)) {
            (b'/', Some(b'/')) => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    out[index] = b' ';
                    index += 1;
                }
            }
            (b'/', Some(b'*')) => {
                while index < bytes.len()
                    && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
                {
                    if bytes[index] != b'\n' {
                        out[index] = b' ';
                    }
                    index += 1;
                }
                for offset in 0..2 {
                    if let Some(byte) = out.get_mut(index + offset) {
                        *byte = b' ';
                    }
                }
                index += 2;
            }
            (quote @ (b'\'' | b'"'), _) => {
                index += 1;
                while index < bytes.len() && bytes[index] != quote && bytes[index] != b'\n' {
                    if bytes[index] == b'\\' {
                        out[index] = b' ';
                        index += 1;
                    }
                    if let Some(byte) = out.get_mut(index) {
                        *byte = b' ';
                    }
                    index += 1;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Razor and Blazor: the file is a component named by its stem; `@inherits`, `@model` and
/// `@layout` types, component tags, partial views, `@code` block methods and calls.
fn razor(path: &str, text: &str) -> Facts {
    static DIRECTIVE: OnceLock<Option<Regex>> = OnceLock::new();
    static TAG: OnceLock<Option<Regex>> = OnceLock::new();
    static PARTIAL: OnceLock<Option<Regex>> = OnceLock::new();
    static METHOD: OnceLock<Option<Regex>> = OnceLock::new();
    let mut facts = Facts::default();
    let stem = path.rsplit('/').next().unwrap_or(path);
    let stem = stem.split('.').next().unwrap_or(stem);
    let component = push_symbol(&mut facts, SymKind::Class, stem, 1, stem);
    facts.symbols[component as usize].end_line = text.lines().count().max(1) as u32;
    let (Some(directive), Some(tag), Some(partial), Some(method)) = (
        regex(
            &DIRECTIVE,
            r"(?m)^\s*@(inherits|model|layout|implements)\s+([\w.<>]+)",
        ),
        regex(&TAG, r"<([A-Z][A-Za-z0-9_]*)[\s/>]"),
        regex(
            &PARTIAL,
            r#"(?:Partial(?:Async)?\(\s*"|<partial\s+name="|RenderPartial(?:Async)?\(\s*")([~/\w.\-]+)""#,
        ),
        regex(
            &METHOD,
            r"(?m)^\s*(?:(?:public|private|protected|internal|static|async|override|virtual)\s+)+[\w<>,.\[\]?\s]*?\b(\w+)\s*\([^;{]*\)\s*\{",
        ),
    ) else {
        return facts;
    };
    for found in directive.captures_iter(text) {
        let (Some(whole), Some(kind), Some(name)) = (found.get(0), found.get(1), found.get(2))
        else {
            continue;
        };
        let line = line_of(text, whole.start());
        let simple = name.as_str().split('<').next().unwrap_or_default();
        let simple = simple.rsplit('.').next().unwrap_or(simple);
        let ref_kind = match kind.as_str() {
            "inherits" => RefKind::Inherit,
            "implements" => RefKind::Implement,
            _ => RefKind::Type,
        };
        push_ref(&mut facts, ref_kind, simple, None, line);
    }
    for found in method.captures_iter(text) {
        let (Some(whole), Some(name)) = (found.get(0), found.get(1)) else {
            continue;
        };
        if C_LIKE_KEYWORDS.contains(&name.as_str()) {
            continue;
        }
        let offset = whole.start() + whole.as_str().len() - whole.as_str().trim_start().len();
        let line = line_of(text, offset);
        let id = push_symbol(
            &mut facts,
            SymKind::Method,
            name.as_str(),
            line,
            text[offset..]
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches('{'),
        );
        facts.symbols[id as usize].end_line = block_end(text, offset);
        facts.symbols[id as usize].owner = stem.into();
    }
    for found in tag.captures_iter(text) {
        let Some(name) = found.get(1) else { continue };
        push_ref(
            &mut facts,
            RefKind::Call,
            name.as_str(),
            None,
            line_of(text, name.start()),
        );
    }
    for found in partial.captures_iter(text) {
        let Some(name) = found.get(1) else { continue };
        let line = line_of(text, name.start());
        let view = name.as_str().trim_start_matches("~/");
        let target = if view.ends_with(".cshtml") {
            view.to_owned()
        } else {
            format!("{view}.cshtml")
        };
        push_ref(&mut facts, RefKind::Path, &target, None, line);
    }
    let code: String = strip_c_comments(text);
    static CODE_CALL: OnceLock<Option<Regex>> = OnceLock::new();
    if let Some(call) = regex(&CODE_CALL, r"(?:@|\b)([A-Za-z_]\w*)\s*\(") {
        for found in call.captures_iter(&code) {
            let Some(name) = found.get(1) else { continue };
            let line = line_of(&code, name.start());
            let declared_here = facts
                .symbols
                .iter()
                .any(|symbol| symbol.line == line && *symbol.name == *name.as_str());
            if !declared_here
                && !C_LIKE_KEYWORDS.contains(&name.as_str())
                && name.as_str().chars().next().is_some_and(char::is_uppercase)
            {
                push_ref(&mut facts, RefKind::Call, name.as_str(), None, line);
            }
        }
    }
    facts
}

/// Blade view name (`layouts.app`) as a path under `resources/views/`.
fn blade_view(name: &str) -> String {
    format!("resources/views/{}.blade.php", name.replace('.', "/"))
}

/// Blade: `@extends`, `@include*`, `@component`, `@each` views and `<x-name>` components as
/// path references; `@section`/`@push` blocks as symbols, `@yield`/`@stack` as their uses.
fn blade(text: &str) -> Facts {
    static VIEW: OnceLock<Option<Regex>> = OnceLock::new();
    static SECTION: OnceLock<Option<Regex>> = OnceLock::new();
    static YIELD: OnceLock<Option<Regex>> = OnceLock::new();
    static COMPONENT: OnceLock<Option<Regex>> = OnceLock::new();
    let mut facts = Facts::default();
    let (Some(view), Some(section), Some(yield_), Some(component)) = (
        regex(
            &VIEW,
            r#"@(?:extends|include|includeIf|includeWhen|includeFirst|includeUnless|component|each)\(\s*(?:\[\s*)?['"]([\w.\-/:]+)['"]"#,
        ),
        regex(
            &SECTION,
            r#"@(?:section|push|prepend)\(\s*['"]([\w.\-]+)['"]"#,
        ),
        regex(&YIELD, r#"@(?:yield|stack)\(\s*['"]([\w.\-]+)['"]"#),
        regex(&COMPONENT, r"<x-([\w.\-]+)[\s/>]"),
    ) else {
        return facts;
    };
    for found in section.captures_iter(text) {
        let Some(name) = found.get(1) else { continue };
        let line = line_of(text, name.start());
        push_symbol(
            &mut facts,
            SymKind::Const,
            name.as_str(),
            line,
            found.get(0).map_or("", |whole| whole.as_str()),
        );
    }
    for found in view.captures_iter(text) {
        let Some(name) = found.get(1) else { continue };
        push_ref(
            &mut facts,
            RefKind::Path,
            &blade_view(name.as_str()),
            None,
            line_of(text, name.start()),
        );
    }
    for found in component.captures_iter(text) {
        let Some(name) = found.get(1) else { continue };
        let view = format!("components.{}", name.as_str());
        push_ref(
            &mut facts,
            RefKind::Path,
            &blade_view(&view),
            None,
            line_of(text, name.start()),
        );
    }
    for found in yield_.captures_iter(text) {
        let Some(name) = found.get(1) else { continue };
        push_ref(
            &mut facts,
            RefKind::Value,
            name.as_str(),
            None,
            line_of(text, name.start()),
        );
    }
    facts
}

/// EJS: `include('partials/header')` (and the legacy `include partials/header`) as path
/// references relative to the template, `.ejs` implied.
fn ejs(text: &str) -> Facts {
    static INCLUDE: OnceLock<Option<Regex>> = OnceLock::new();
    let mut facts = Facts::default();
    let Some(include) = regex(&INCLUDE, r#"include\(?\s*['"]?([\w./\-]+)['"]?"#) else {
        return facts;
    };
    for found in include.captures_iter(text) {
        let Some(name) = found.get(1) else { continue };
        let target = if name.as_str().ends_with(".ejs") {
            name.as_str().to_owned()
        } else {
            format!("{}.ejs", name.as_str())
        };
        push_ref(
            &mut facts,
            RefKind::Path,
            &target,
            None,
            line_of(text, name.start()),
        );
    }
    facts
}

struct Opening {
    kind: SymKind,
    name: String,
    signature: String,
}

/// Jinja: `extends`, `include`, `import` and `from` templates as path references, `block` and
/// `macro` definitions as symbols, macro calls.
fn jinja(text: &str) -> Facts {
    static TEMPLATE: OnceLock<Option<Regex>> = OnceLock::new();
    static DEFINITION: OnceLock<Option<Regex>> = OnceLock::new();
    static CALL: OnceLock<Option<Regex>> = OnceLock::new();
    let mut facts = Facts::default();
    let (Some(template), Some(definition), Some(call)) = (
        regex(
            &TEMPLATE,
            r#"\{%-?\s*(?:extends|include|import|from)\s+['"]([^'"]+)['"]"#,
        ),
        regex(&DEFINITION, r"\{%-?\s*(block|macro)\s+(\w+)"),
        regex(&CALL, r"(?:\{\{-?|\{%-?\s*call)\s*(?:(\w+)\.)?(\w+)\s*\("),
    ) else {
        return facts;
    };
    static END: OnceLock<Option<Regex>> = OnceLock::new();
    let end = regex(&END, r"\{%-?\s*end(block|macro)\b");
    let mut open: Vec<u32> = Vec::new();
    // Openings (`block`, `macro`) with kind, name and signature; `None` closes the innermost.
    let mut events: Vec<(usize, Option<Opening>)> = Vec::new();
    for found in definition.captures_iter(text) {
        let (Some(whole), Some(keyword), Some(name)) = (found.get(0), found.get(1), found.get(2))
        else {
            continue;
        };
        let kind = if keyword.as_str() == "macro" {
            SymKind::Fn
        } else {
            SymKind::Const
        };
        events.push((
            whole.start(),
            Some(Opening {
                kind,
                name: name.as_str().to_owned(),
                signature: whole.as_str().to_owned(),
            }),
        ));
    }
    if let Some(end) = end {
        events.extend(end.find_iter(text).map(|found| (found.start(), None)));
    }
    events.sort_by_key(|event| event.0);
    for (offset, event) in events {
        let line = line_of(text, offset);
        match event {
            Some(Opening {
                kind,
                name,
                signature,
            }) => {
                let id = push_symbol(&mut facts, kind, &name, line, &signature);
                facts.symbols[id as usize].parent = open.last().copied();
                facts.symbols[id as usize].end_line = facts.lines.max(u32::MAX >> 1);
                open.push(id);
            }
            None => {
                if let Some(closed) = open.pop() {
                    facts.symbols[closed as usize].end_line = line;
                }
            }
        }
    }
    let last = line_of(text, text.len());
    for symbol in &mut facts.symbols {
        symbol.end_line = symbol.end_line.min(last).max(symbol.line);
    }
    for found in template.captures_iter(text) {
        let Some(name) = found.get(1) else { continue };
        push_ref(
            &mut facts,
            RefKind::Path,
            name.as_str(),
            None,
            line_of(text, name.start()),
        );
    }
    for found in call.captures_iter(text) {
        let Some(name) = found.get(2) else { continue };
        let qualifier = found.get(1).map(|qualifier| qualifier.as_str());
        push_ref(
            &mut facts,
            RefKind::Call,
            name.as_str(),
            qualifier,
            line_of(text, name.start()),
        );
    }
    facts
}

/// COBOL (fixed or free format): `PROGRAM-ID` as entry, procedure division paragraphs and
/// sections, `PERFORM` and `CALL 'PROG'` calls, `COPY` copybooks as path references.
fn cobol(text: &str) -> Facts {
    static PROGRAM: OnceLock<Option<Regex>> = OnceLock::new();
    static PARAGRAPH: OnceLock<Option<Regex>> = OnceLock::new();
    static PERFORM: OnceLock<Option<Regex>> = OnceLock::new();
    static CALL: OnceLock<Option<Regex>> = OnceLock::new();
    static COPY: OnceLock<Option<Regex>> = OnceLock::new();
    let mut facts = Facts::default();
    let (Some(program), Some(paragraph), Some(perform), Some(call), Some(copy)) = (
        regex(&PROGRAM, r"(?i)^\s*PROGRAM-ID\.\s*([\w\-]+)"),
        regex(&PARAGRAPH, r"(?i)^([A-Z0-9][\w\-]*)(\s+SECTION)?\s*\.\s*$"),
        regex(&PERFORM, r"(?i)\bPERFORM\s+([A-Z0-9][\w\-]*)"),
        regex(&CALL, r#"(?i)\bCALL\s+['"]([\w\-]+)['"]"#),
        regex(&COPY, r#"(?i)\bCOPY\s+['"]?([\w\-]+)['"]?"#),
    ) else {
        return facts;
    };
    let mut in_procedure = false;
    let mut current: Option<u32> = None;
    for (index, raw) in text.lines().enumerate() {
        let number = index as u32 + 1;
        // Fixed format: columns 1-6 sequence numbers, column 7 indicator (`*`, `/` comments).
        let fixed = raw.len() > 7 && raw[..6].chars().all(|ch| ch.is_ascii_digit() || ch == ' ');
        let line = if fixed {
            if matches!(raw.as_bytes()[6], b'*' | b'/') {
                continue;
            }
            raw.get(7..raw.len().min(72)).unwrap_or_default()
        } else {
            if raw.trim_start().starts_with("*>") {
                continue;
            }
            raw
        };
        let upper = line.to_ascii_uppercase();
        if let Some(found) = program.captures(line).and_then(|found| found.get(1)) {
            let id = push_symbol(
                &mut facts,
                SymKind::Mod,
                &found.as_str().to_ascii_uppercase(),
                number,
                line.trim(),
            );
            facts.symbols[id as usize].flags |= FLAG_ENTRY;
            facts.symbols[id as usize].end_line = text.lines().count() as u32;
            continue;
        }
        if upper.contains("PROCEDURE DIVISION") {
            in_procedure = true;
            continue;
        }
        let area_a =
            line.starts_with(|ch: char| !ch.is_whitespace()) || !fixed && !line.starts_with("    ");
        if in_procedure
            && area_a
            && let Some(found) = paragraph
                .captures(line.trim_end())
                .and_then(|found| found.get(1))
            && !matches!(
                found.as_str().to_ascii_uppercase().as_str(),
                "END-IF" | "END-PERFORM" | "EXIT" | "STOP" | "GOBACK"
            )
        {
            if let Some(previous) = current {
                facts.symbols[previous as usize].end_line = number
                    .saturating_sub(1)
                    .max(facts.symbols[previous as usize].line);
            }
            let id = push_symbol(
                &mut facts,
                SymKind::Fn,
                &found.as_str().to_ascii_uppercase(),
                number,
                line.trim(),
            );
            facts.symbols[id as usize].end_line = text.lines().count() as u32;
            current = Some(id);
            continue;
        }
        for found in perform.captures_iter(line).filter_map(|found| found.get(1)) {
            let name = found.as_str().to_ascii_uppercase();
            if !matches!(
                name.as_str(),
                "UNTIL" | "VARYING" | "WITH" | "TEST" | "TIMES"
            ) {
                push_ref(&mut facts, RefKind::Call, &name, None, number);
            }
        }
        for found in call.captures_iter(line).filter_map(|found| found.get(1)) {
            push_ref(
                &mut facts,
                RefKind::Call,
                &found.as_str().to_ascii_uppercase(),
                None,
                number,
            );
        }
        for found in copy.captures_iter(line).filter_map(|found| found.get(1)) {
            push_ref(
                &mut facts,
                RefKind::Path,
                &format!("{}.cpy", found.as_str()),
                None,
                number,
            );
        }
    }
    facts
}

/// XAML: `x:Class` as the class (its code-behind `.xaml.cs` as a path reference), `x:Name`
/// elements, event handler attributes as calls, resource dictionary and image sources as path
/// references.
fn xaml(path: &str, text: &str) -> Facts {
    static CLASS: OnceLock<Option<Regex>> = OnceLock::new();
    static NAME: OnceLock<Option<Regex>> = OnceLock::new();
    static HANDLER: OnceLock<Option<Regex>> = OnceLock::new();
    static SOURCE: OnceLock<Option<Regex>> = OnceLock::new();
    let mut facts = Facts::default();
    let (Some(class), Some(name), Some(handler), Some(source)) = (
        regex(&CLASS, r#"x:Class="([\w.]+)""#),
        regex(&NAME, r#"\bx:Name="(\w+)""#),
        regex(
            &HANDLER,
            r#"\b(Click|Loaded|Unloaded|Checked|Unchecked|SelectionChanged|TextChanged|MouseDown|MouseUp|KeyDown|KeyUp|Closing|Closed|Tapped|ValueChanged|Toggled|Executed|CanExecute|Activated|Opened|PointerPressed|Drop|DragOver|GotFocus|LostFocus)="(\w+)""#,
        ),
        regex(
            &SOURCE,
            r#"\b(?:Source|ImageSource|Icon)="([\w./\-]+\.(?:xaml|png|jpg|jpeg|gif|ico|svg|bmp))""#,
        ),
    ) else {
        return facts;
    };
    let class_name = class
        .captures(text)
        .and_then(|found| found.get(1))
        .map(|found| {
            let full = found.as_str();
            (
                full.rsplit('.').next().unwrap_or(full).to_owned(),
                line_of(text, found.start()),
            )
        });
    if let Some((simple, line)) = &class_name {
        let id = push_symbol(&mut facts, SymKind::Class, simple, *line, simple);
        facts.symbols[id as usize].end_line = text.lines().count().max(1) as u32;
        let file = path.rsplit('/').next().unwrap_or(path);
        push_ref(
            &mut facts,
            RefKind::Path,
            &format!("{file}.cs"),
            None,
            *line,
        );
    }
    for found in name.captures_iter(text).filter_map(|found| found.get(1)) {
        push_symbol(
            &mut facts,
            SymKind::Var,
            found.as_str(),
            line_of(text, found.start()),
            found.as_str(),
        );
    }
    // Handlers live in the code-behind partial class; structural calls resolve by unique name.
    for found in handler.captures_iter(text).filter_map(|found| found.get(2)) {
        push_ref(
            &mut facts,
            RefKind::Call,
            found.as_str(),
            None,
            line_of(text, found.start()),
        );
    }
    for found in source.captures_iter(text).filter_map(|found| found.get(1)) {
        push_ref(
            &mut facts,
            RefKind::Path,
            found.as_str().trim_start_matches('/'),
            None,
            line_of(text, found.start()),
        );
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_comments_and_string_contents_are_blanked_with_line_structure_intact() {
        let text = "a(); // b()\n/* c()\n d() */ e();\nf(\"// g() /* h */\");\n/* open";
        let stripped = strip_c_comments(text);
        assert_eq!(stripped.len(), text.len());
        assert_eq!(stripped.lines().count(), text.lines().count());
        for gone in ["b()", "c()", "d()", "g()", "h", "open"] {
            assert!(!stripped.contains(gone), "{gone}: {stripped}");
        }
        for kept in ["a();", "e();", "f(\""] {
            assert!(stripped.contains(kept), "{kept}: {stripped}");
        }
    }

    fn summary(facts: &Facts) -> (Vec<String>, Vec<String>) {
        let symbols = facts
            .symbols
            .iter()
            .map(|symbol| {
                let parent = symbol.parent.map_or("-".to_owned(), |p| {
                    facts.symbols[p as usize].name.to_string()
                });
                format!(
                    "{} {} {}-{} <{parent} f{}",
                    symbol.kind().id(),
                    symbol.name,
                    symbol.line,
                    symbol.end_line,
                    symbol.flags
                )
            })
            .collect();
        let refs = facts
            .refs
            .iter()
            .map(|reference| {
                let qualifier = reference
                    .qualifier
                    .as_deref()
                    .map_or(String::new(), |q| format!("{q}."));
                format!(
                    "{} {:?} {qualifier}{}",
                    reference.line, reference.kind, reference.name
                )
            })
            .collect();
        (symbols, refs)
    }

    #[test]
    fn apex_classes_methods_tests_and_calls() {
        let text = "@isTest\npublic with sharing class StoreTest extends BaseTest implements Schedulable {\n    static testMethod void opens() {\n        Store s = new Store();\n        s.open('x'); // helper(1)\n        System.assert(StoreService.count() > 0);\n    }\n}\n";
        let (symbols, refs) = summary(&apex(text));
        assert_eq!(
            symbols,
            [
                "class StoreTest 1-8 <- f1",
                "method opens 3-7 <StoreTest f1"
            ]
        );
        assert_eq!(
            refs,
            [
                "1 Inherit BaseTest",
                "1 Implement Schedulable",
                "4 Call Store",
                "5 Call s.open",
                "6 Call System.assert",
                "6 Call StoreService.count"
            ]
        );
        let (symbols, _) = summary(&apex(
            "trigger AccountTrigger on Account (before insert) {\n    AccountService.handle(Trigger.new);\n}\n",
        ));
        assert_eq!(symbols, ["fn AccountTrigger 1-3 <- f2"]);
    }

    #[test]
    fn razor_components_partials_and_code() {
        let text = "@page \"/store\"\n@inherits StoreBase\n<StoreGrid Items=\"rows\" />\n<partial name=\"_Header\" />\n@code {\n    private void Load() { Refresh(); }\n}\n";
        let (symbols, refs) = summary(&razor("Pages/Store.razor", text));
        assert_eq!(
            symbols,
            ["class Store 1-7 <- f0", "method Load 6-6 <Store f0"]
        );
        assert_eq!(
            refs,
            [
                "2 Inherit StoreBase",
                "3 Call StoreGrid",
                "4 Path _Header.cshtml",
                "6 Call Refresh"
            ]
        );
    }

    #[test]
    fn blade_ejs_and_jinja_templates() {
        let (symbols, refs) = summary(&blade(
            "@extends('layouts.app')\n@section('content')\n@include('partials.nav')\n<x-alert type=\"error\"/>\n@endsection\n@yield('scripts')\n",
        ));
        assert_eq!(symbols, ["const content 2-2 <- f0"]);
        assert_eq!(
            refs,
            [
                "1 Path resources/views/layouts/app.blade.php",
                "3 Path resources/views/partials/nav.blade.php",
                "4 Path resources/views/components/alert.blade.php",
                "6 Value scripts"
            ]
        );
        let (_, refs) = summary(&ejs(
            "<%- include('partials/header') %>\n<% include footer %>\n",
        ));
        assert_eq!(refs, ["1 Path partials/header.ejs", "2 Path footer.ejs"]);
        let (symbols, refs) = summary(&jinja(
            "{% extends \"base.html\" %}\n{% import 'macros.html' as m %}\n{% block content %}\n{{ m.card(item) }}\n{% macro row(x) %}{{ x }}{% endmacro %}\n{% endblock %}\n",
        ));
        assert_eq!(
            symbols,
            ["const content 3-6 <- f0", "fn row 5-5 <content f0"]
        );
        assert_eq!(
            refs,
            ["1 Path base.html", "2 Path macros.html", "4 Call m.card"]
        );
    }

    #[test]
    fn cobol_paragraphs_performs_calls_and_copybooks() {
        let text = "000100 IDENTIFICATION DIVISION.\n000200 PROGRAM-ID. STORE.\n000300* comment PERFORM NOPE\n000400 PROCEDURE DIVISION.\n000500 MAIN-PARA.\n000600     PERFORM LOAD-ROWS\n000700     CALL 'AUDIT' USING WS-ROW\n000800     STOP RUN.\n000900 LOAD-ROWS.\n001000     COPY ROWDEF.\n";
        let (symbols, refs) = summary(&cobol(text));
        assert_eq!(
            symbols,
            [
                "mod STORE 2-10 <- f2",
                "fn MAIN-PARA 5-8 <STORE f0",
                "fn LOAD-ROWS 9-10 <STORE f0"
            ]
        );
        assert_eq!(
            refs,
            ["6 Call LOAD-ROWS", "7 Call AUDIT", "10 Path ROWDEF.cpy"]
        );
    }

    #[test]
    fn xaml_handlers_and_paths() {
        let text = "<Window x:Class=\"Store.MainWindow\"\n        Loaded=\"OnLoaded\">\n  <Button x:Name=\"SaveButton\" Click=\"OnSave\"/>\n  <Image Source=\"Assets/logo.png\"/>\n</Window>\n";
        let (symbols, refs) = summary(&xaml("Views/MainWindow.xaml", text));
        assert_eq!(
            symbols,
            [
                "class MainWindow 1-5 <- f0",
                "var SaveButton 3-3 <MainWindow f0"
            ]
        );
        assert_eq!(
            refs,
            [
                "1 Path MainWindow.xaml.cs",
                "2 Call OnLoaded",
                "3 Call OnSave",
                "4 Path Assets/logo.png"
            ]
        );
    }
}
