//! Pascal and Delphi: class body declarations of `TType.Method` implementations, `program`
//! entrypoints, DUnit `Test*` methods in test files; `.dfm`/`.lfm` forms as components with
//! their event handlers.

use super::Pack;
use crate::domain::facts::{
    self, FLAG_DECLARATION, FLAG_TEST, Facts, Import, NO_ARITY, Ref, RefKind, SymKind, Symbol,
};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Pascal,
    language: |_| tree_sitter_pascal::LANGUAGE.into(),
    query: include_str!("../queries/pascal.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin: |name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "inc"
                    | "dec"
                    | "length"
                    | "setlength"
                    | "high"
                    | "low"
                    | "assigned"
                    | "freeandnil"
                    | "format"
                    | "inttostr"
                    | "strtoint"
                    | "writeln"
                    | "write"
                    | "readln"
                    | "exit"
                    | "result"
                    | "self"
                    | "inherited"
                    | "raise"
                    | "trim"
                    | "uppercase"
                    | "lowercase"
                    | "copy"
                    | "pos"
                    | "ord"
                    | "chr"
                    | "getenvironmentvariable"
                    | "create"
                    | "free"
                    | "integer"
                    | "string"
                    | "boolean"
                    | "cardinal"
            )
        },
    },
};

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let callable = matches!(definition.kind, SymKind::Fn | SymKind::Method);
    let lower = definition.text.to_ascii_lowercase();
    if callable && !lower.contains("begin") && !lower.contains(" asm") {
        return FLAG_DECLARATION;
    }
    // Forward class declarations (`TCore = class;`) announce the type defined later;
    // `EError = class(Exception);` is a complete definition.
    if definition.kind.is_type() {
        let forward = ["class", "interface", "dispinterface"]
            .iter()
            .any(|keyword| {
                lower
                    .split_once('=')
                    .map(|(_, rest)| rest.trim().trim_end_matches(';').trim_end())
                    .is_some_and(|rest| rest == *keyword)
            });
        if forward {
            return FLAG_DECLARATION;
        }
    }
    let test_file = context.path.to_ascii_lowercase().contains("test");
    if callable && test_file && definition.name.starts_with("Test") {
        return FLAG_TEST;
    }
    0
}

/// Text `.dfm`/`.lfm` form: `object Name: TClass` components as variables (nested by
/// `object`/`end`), `OnEvent = Handler` assignments as calls of the form class method
/// `TForm.Handler` from its component, and the unit with the same stem as an import. Binary
/// forms are skipped.
pub fn form(path: &str, text: &str) -> Facts {
    let mut facts = Facts::default();
    if text.as_bytes().first() == Some(&0xff) {
        return facts;
    }
    if let Some((stem, _)) = path.rsplit_once('.') {
        let file = stem.rsplit('/').next().unwrap_or(stem);
        facts.imports.push(Import {
            spec: format!("{file}.pas"),
            line: 1,
            ..Import::default()
        });
    }
    let mut open: Vec<u32> = Vec::new();
    let mut form_class: Option<String> = None;
    let line_count = text.lines().count() as u32;
    for (index, line) in text.lines().enumerate() {
        let number = index as u32 + 1;
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        let header = ["object ", "inherited ", "inline "]
            .iter()
            .find_map(|keyword| {
                lower
                    .starts_with(keyword)
                    .then(|| &trimmed[keyword.len()..])
            });
        if let Some(header) = header {
            let (name, class) = header.split_once(':').unwrap_or((header, ""));
            let id = facts.symbols.len() as u32;
            facts.symbols.push(Symbol {
                kind: Some(SymKind::Var),
                name: name.trim().to_owned(),
                parent: open.last().copied(),
                line: number,
                end_line: line_count,
                signature: facts::clip_space(trimmed, 160),
                exported: true,
                ..Symbol::default()
            });
            let class = class.trim();
            if form_class.is_none() && !class.is_empty() {
                form_class = Some(class.split_whitespace().next().unwrap_or(class).to_owned());
            }
            if !class.is_empty() {
                facts.refs.push(Ref {
                    kind: RefKind::Type,
                    name: class.split_whitespace().next().unwrap_or(class).to_owned(),
                    qualifier: None,
                    scope: Some(id),
                    line: number,
                    arity: NO_ARITY,
                });
            }
            open.push(id);
            continue;
        }
        if lower == "end" {
            if let Some(closed) = open.pop() {
                facts.symbols[closed as usize].end_line = number;
            }
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=')
            && key.trim().starts_with("On")
            && value
                .trim()
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '_')
            && !value.trim().is_empty()
        {
            facts.refs.push(Ref {
                kind: RefKind::Call,
                name: value.trim().to_owned(),
                qualifier: form_class.clone(),
                scope: open.last().copied(),
                line: number,
                arity: NO_ARITY,
            });
        }
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_components_and_handlers() {
        let facts = form(
            "src/Main.dfm",
            "object MainForm: TMainForm\n  Caption = 'Store'\n  object OpenButton: TButton\n    OnClick = OpenButtonClick\n  end\nend\n",
        );
        let symbols: Vec<String> = facts
            .symbols
            .iter()
            .map(|symbol| {
                format!(
                    "{} {}-{} {:?}",
                    symbol.name, symbol.line, symbol.end_line, symbol.parent
                )
            })
            .collect();
        assert_eq!(symbols, ["MainForm 1-6 None", "OpenButton 3-5 Some(0)"]);
        let refs: Vec<String> = facts
            .refs
            .iter()
            .map(|reference| {
                format!(
                    "{:?} {} {:?}",
                    reference.kind, reference.name, reference.scope
                )
            })
            .collect();
        assert_eq!(
            refs,
            [
                "Type TMainForm Some(0)",
                "Type TButton Some(1)",
                "Call OpenButtonClick Some(1)"
            ]
        );
        assert_eq!(facts.refs[2].qualifier.as_deref(), Some("TMainForm"));
        assert_eq!(facts.imports[0].spec, "Main.pas");
        assert!(form("a.dfm", "\u{ff}binary").symbols.is_empty());
    }
}
