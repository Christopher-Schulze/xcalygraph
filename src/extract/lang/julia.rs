//! Julia: names starting with `_` are private; `include` files and `using`/`import` modules.

use super::Pack;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Julia,
    language: |_| tree_sitter_julia::LANGUAGE.into(),
    query: include_str!("../queries/julia.scm"),
    hooks: Hooks {
        exported: |definition| !definition.name.starts_with('_'),
        signature_prefix: |_, _| "",
        flags: |_, _| 0,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin: |name| {
            matches!(
                name,
                "println"
                    | "print"
                    | "length"
                    | "push!"
                    | "pop!"
                    | "get"
                    | "haskey"
                    | "isempty"
                    | "string"
                    | "error"
                    | "throw"
                    | "map"
                    | "filter"
                    | "collect"
                    | "sum"
                    | "typeof"
                    | "isa"
                    | "include"
                    | "convert"
                    | "zeros"
                    | "ones"
                    | "size"
                    | "eltype"
                    | "keys"
                    | "values"
                    | "first"
                    | "last"
                    | "abs"
                    | "max"
                    | "min"
                    | "round"
                    | "parse"
                    | "join"
                    | "split"
                    | "replace"
                    | "Dict"
                    | "Vector"
                    | "Int"
                    | "String"
            )
        },
    },
};
