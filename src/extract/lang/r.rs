//! R: names starting with `.` are private; `source` files and `library` packages.

use super::Pack;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::R,
    language: |_| tree_sitter_r::LANGUAGE.into(),
    query: include_str!("../queries/r.scm"),
    hooks: Hooks {
        exported: |definition| !definition.name.starts_with('.'),
        signature_prefix: |_, _| "",
        flags: |_, _| 0,
        expand_import: engine::single_import,
        builtin: |name| {
            matches!(
                name,
                "c" | "list"
                    | "paste"
                    | "paste0"
                    | "length"
                    | "stop"
                    | "print"
                    | "cat"
                    | "is.null"
                    | "return"
                    | "function"
                    | "library"
                    | "require"
                    | "source"
                    | "names"
                    | "nrow"
                    | "ncol"
                    | "seq_along"
                    | "vapply"
                    | "sapply"
                    | "lapply"
                    | "invisible"
                    | "message"
                    | "warning"
                    | "stopifnot"
                    | "data.frame"
                    | "matrix"
                    | "as.character"
                    | "as.numeric"
                    | "is.na"
                    | "sprintf"
                    | "identical"
                    | "missing"
                    | "test_that"
                    | "expect_equal"
                    | "expect_true"
                    | "expect_false"
                    | "expect_error"
            )
        },
    },
};
