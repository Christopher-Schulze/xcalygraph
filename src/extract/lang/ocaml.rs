//! OCaml: implementations (`.ml`) and interfaces (`.mli`, specifications as declarations).

use super::Pack;
use crate::domain::facts::FLAG_DECLARATION;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks, ImportExpansion};

fn builtin(name: &str) -> bool {
    matches!(
        name,
        "print_endline"
            | "print_string"
            | "print_int"
            | "print_newline"
            | "failwith"
            | "raise"
            | "ignore"
            | "fst"
            | "snd"
            | "not"
            | "ref"
            | "invalid_arg"
            | "string_of_int"
            | "int_of_string"
            | "compare"
            | "min"
            | "max"
            | "incr"
            | "decr"
    )
}

pub static IMPLEMENTATION: Pack = Pack {
    suffixes: &[],
    file_type: FileType::OCaml,
    language: |_| tree_sitter_ocaml::LANGUAGE_OCAML.into(),
    query: include_str!("../queries/ocaml.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |_, _| 0,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin,
    },
};

pub static INTERFACE: Pack = Pack {
    suffixes: &[".mli"],
    file_type: FileType::OCaml,
    language: |_| tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into(),
    query: include_str!("../queries/ocaml_interface.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |_, _| FLAG_DECLARATION,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin,
    },
};
