//! Visual Basic .NET: `Public`, `Protected` and `Friend` visibility, `<Fact>`-style test
//! attributes, `Sub Main` entry. `Imports`, `Inherits` and `Implements` lines are scanned from the
//! text: the grammar misparses import aliases and supertype lines.

use super::Pack;
use super::csharp::{attributes, dotnet_flags, using_import};
use super::java::header;
use crate::domain::facts::{Facts, NO_ARITY, Ref, RefKind, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::VbNet,
    language: |_| tree_sitter_vb_dotnet::LANGUAGE.into(),
    query: include_str!("../queries/vbnet.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(|_, _, _| Vec::new()),
        builtin,
    },
};

fn exported(definition: &Definition) -> bool {
    header(definition).split_whitespace().any(|word| {
        matches!(
            word.to_ascii_lowercase().as_str(),
            "public" | "protected" | "friend"
        )
    }) || matches!(definition.parent, Some((SymKind::Iface, _)))
}

fn flags(definition: &Definition, _context: &FileContext) -> u8 {
    dotnet_flags(definition, &attributes(header(definition), '<', '>'), true)
}

/// Adds the `Imports` lines as imports and `Inherits`/`Implements` lines as supertype references
/// of the innermost type declared around them.
pub fn scan_lines(source: &[u8], facts: &mut Facts) {
    let text = String::from_utf8_lossy(source);
    for (index, line) in text.lines().enumerate() {
        let number = index as u32 + 1;
        let code = line.split('\'').next().unwrap_or_default().trim();
        let mut words = code.splitn(2, char::is_whitespace);
        let keyword = words.next().unwrap_or_default().to_ascii_lowercase();
        let rest = words.next().unwrap_or_default().trim();
        match keyword.as_str() {
            "imports" => {
                facts
                    .imports
                    .extend(using_import(&format!("using {rest}"), number, false))
            }
            "inherits" | "implements" => {
                let scope = facts
                    .symbols
                    .iter()
                    .enumerate()
                    .filter(|(_, symbol)| {
                        symbol.kind().is_type()
                            && symbol.line <= number
                            && number <= symbol.end_line
                    })
                    .max_by_key(|(_, symbol)| symbol.line)
                    .map(|(position, _)| position as u32);
                let kind = if keyword == "inherits" {
                    RefKind::Inherit
                } else {
                    RefKind::Implement
                };
                for name in rest.split(',') {
                    let name = name.split("(Of").next().unwrap_or_default().trim();
                    let name = name.rsplit('.').next().unwrap_or_default();
                    if !name.is_empty() {
                        facts.refs.push(Ref {
                            kind,
                            name: name.to_owned(),
                            qualifier: None,
                            scope,
                            line: number,
                            arity: NO_ARITY,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// VB.NET keywords and `System` names visible everywhere.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "Console"
            | "Math"
            | "String"
            | "Object"
            | "Exception"
            | "Environment"
            | "CType"
            | "CStr"
            | "CInt"
            | "DirectCast"
            | "TryCast"
            | "Nothing"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_and_supertypes_come_from_lines() {
        let mut facts = Facts::default();
        facts.symbols.push(crate::domain::facts::Symbol {
            kind: Some(SymKind::Class),
            name: "C".to_owned(),
            line: 3,
            end_line: 6,
            ..crate::domain::facts::Symbol::default()
        });
        let source = b"Imports App.Store\nImports Repo = App.Store.IRepository\nPublic Class C\n    Inherits BaseC ' base\n    Implements IRepo, IOther(Of Integer)\nEnd Class\n";
        scan_lines(source, &mut facts);
        let imports: Vec<(&str, Option<&str>)> = facts
            .imports
            .iter()
            .map(|import| (import.spec.as_str(), import.alias.as_deref()))
            .collect();
        assert_eq!(imports, [("App.Store", None), ("App.Store", Some("Repo"))]);
        let refs: Vec<(RefKind, &str, Option<u32>)> = facts
            .refs
            .iter()
            .map(|reference| (reference.kind, reference.name.as_str(), reference.scope))
            .collect();
        assert_eq!(
            refs,
            [
                (RefKind::Inherit, "BaseC", Some(0)),
                (RefKind::Implement, "IRepo", Some(0)),
                (RefKind::Implement, "IOther", Some(0))
            ]
        );
    }
}
