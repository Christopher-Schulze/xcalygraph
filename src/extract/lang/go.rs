//! Go: package-level visibility by capitalization, `_test.go` test functions, `main` entry.

use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks, ImportExpansion};
use tree_sitter::Node;

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Go,
    language: |_| tree_sitter_go::LANGUAGE.into(),
    query: include_str!("../queries/go.scm"),
    hooks: Hooks {
        exported,
        signature_prefix,
        flags,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin,
    },
};

/// Nominal type of a literal receiver, borrowing the existing AST instead of its initializer.
pub(in crate::extract) fn composite_receiver_type<'tree>(
    mut node: Node<'tree>,
    source: &[u8],
) -> Option<Node<'tree>> {
    if node.has_error() || node.is_missing() {
        return None;
    }
    loop {
        node = match node.kind() {
            "parenthesized_expression" => node.named_child(0)?,
            "unary_expression"
                if node
                    .child_by_field_name("operator")?
                    .utf8_text(source)
                    .ok()?
                    == "&" =>
            {
                node.child_by_field_name("operand")?
            }
            "composite_literal" => {
                let mut nominal = node.child_by_field_name("type")?;
                if nominal.kind() == "generic_type" {
                    nominal = nominal.child_by_field_name("type")?;
                }
                return matches!(nominal.kind(), "type_identifier" | "qualified_type")
                    .then_some(nominal);
            }
            _ => return None,
        };
    }
}

/// Predeclared Go functions and types.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "append"
            | "cap"
            | "clear"
            | "close"
            | "complex"
            | "copy"
            | "delete"
            | "imag"
            | "len"
            | "make"
            | "max"
            | "min"
            | "new"
            | "panic"
            | "print"
            | "println"
            | "real"
            | "recover"
            | "bool"
            | "byte"
            | "complex64"
            | "complex128"
            | "error"
            | "float32"
            | "float64"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "rune"
            | "string"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
            | "any"
            | "comparable"
    )
}

fn exported(definition: &Definition) -> bool {
    definition.name.starts_with(|ch: char| ch.is_uppercase())
}

fn signature_prefix(kind: SymKind, text: &str) -> &'static str {
    match kind {
        SymKind::Struct | SymKind::Iface | SymKind::Alias if !text.starts_with("type") => "type ",
        SymKind::Const if !text.starts_with("const") => "const ",
        SymKind::Var if !text.starts_with("var") => "var ",
        _ => "",
    }
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let mut flags = 0;
    let test_prefix = ["Test", "Benchmark", "Fuzz", "Example"]
        .iter()
        .any(|prefix| definition.name.starts_with(prefix));
    if definition.kind == SymKind::Fn && context.path.ends_with("_test.go") && test_prefix {
        flags |= FLAG_TEST;
    }
    if definition.kind == SymKind::Fn
        && definition.name == "main"
        && context.package == Some("main")
    {
        flags |= FLAG_ENTRY;
    }
    flags
}

#[cfg(test)]
mod tests {
    #[test]
    fn composite_literal_receivers_keep_their_nominal_type() -> std::io::Result<()> {
        use crate::domain::graph::{Confidence, EdgeKind};
        use crate::infra::testdir::TestDir;

        let root = TestDir::new("go-composite-receivers")?;
        root.write(
            "lib.go",
            concat!(
                "package example\n",
                "type Disk struct { ID int }\n",
                "type Other struct {}\n",
                "type Box[T any] struct { Value T }\n",
                "func (disk Disk) Read() int { return disk.ID }\n",
                "func (other Other) Read() int { return 0 }\n",
                "func (box Box[T]) Read() T { return box.Value }\n",
                "func direct() int { return Disk{ID:1}.Read() }\n",
                "func parenthesized() int { return (Disk{ID:2}).Read() }\n",
                "func pointer() int { return (&Disk{ID:3}).Read() }\n",
                "func variable() int { disk := Disk{ID:4}; return disk.Read() }\n",
                "func different() int { return Other{}.Read() }\n",
                "func makeDisk() Disk { return Disk{ID:5} }\n",
                "func called() int { return makeDisk().Read() }\n",
                "func generic() int { return Box[int]{Value:9}.Read() }\n",
            ),
        )?;
        let graph = crate::resolve::tests::graph_of(root.path())?;
        let mut calls: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls)
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                if target.name != "Read" {
                    return None;
                }
                let &(owner, _) = graph
                    .members
                    .iter()
                    .find(|&&(_, member)| member == edge.target)?;
                let owner = graph.symbols.get(owner as usize)?;
                Some((source.name.as_str(), owner.name.as_str(), edge.confidence))
            })
            .collect();
        calls.sort_unstable();
        assert_eq!(
            calls,
            [
                ("called", "Disk", Confidence::Inferred),
                ("different", "Other", Confidence::Exact),
                ("direct", "Disk", Confidence::Exact),
                ("generic", "Box", Confidence::Exact),
                ("parenthesized", "Disk", Confidence::Exact),
                ("pointer", "Disk", Confidence::Exact),
                ("variable", "Disk", Confidence::Exact),
            ],
            "{}",
            crate::resolve::tests::render(&graph)
        );
        Ok(())
    }
}
