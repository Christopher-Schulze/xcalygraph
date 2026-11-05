//! PowerShell: functions, classes and methods; modules, dot-sourced and invoked scripts as
//! imports; `$env:NAME` reads.

use super::Pack;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::PowerShell,
    language: |_| tree_sitter_powershell::LANGUAGE.into(),
    query: include_str!("../queries/powershell.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |_, _| 0,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin: |name| {
            let lower = name.to_ascii_lowercase();
            matches!(
                lower.as_str(),
                "write-host"
                    | "write-output"
                    | "write-error"
                    | "write-verbose"
                    | "write-warning"
                    | "get-item"
                    | "get-childitem"
                    | "set-location"
                    | "join-path"
                    | "test-path"
                    | "new-object"
                    | "import-module"
                    | "using"
                    | "describe"
                    | "it"
                    | "context"
                    | "should"
                    | "beforeall"
                    | "beforeeach"
                    | "aftereach"
                    | "afterall"
                    | "param"
                    | "foreach-object"
                    | "where-object"
                    | "select-object"
                    | "out-null"
            )
        },
    },
};
