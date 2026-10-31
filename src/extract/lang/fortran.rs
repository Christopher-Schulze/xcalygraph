//! Fortran: `program` units as entrypoints; module procedures visible through `use`.

use super::Pack;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Fortran,
    language: |_| tree_sitter_fortran::LANGUAGE.into(),
    query: include_str!("../queries/fortran.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |_, _| 0,
        expand_import: engine::single_import,
        builtin: |name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "size"
                    | "allocated"
                    | "present"
                    | "trim"
                    | "len"
                    | "len_trim"
                    | "abs"
                    | "sqrt"
                    | "max"
                    | "min"
                    | "mod"
                    | "real"
                    | "int"
                    | "dble"
                    | "sum"
                    | "maxval"
                    | "minval"
                    | "associated"
                    | "adjustl"
                    | "get_environment_variable"
                    | "get_command_argument"
                    | "exp"
                    | "log"
                    | "sin"
                    | "cos"
                    | "huge"
                    | "tiny"
                    | "epsilon"
                    | "kind"
                    | "allocate"
                    | "deallocate"
                    | "nint"
                    | "floor"
            )
        },
    },
};
