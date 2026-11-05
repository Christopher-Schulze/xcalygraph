//! Robot Framework: test cases are tests, keywords are functions; `Resource` and `Library`
//! settings are imports.

use super::Pack;
use crate::domain::facts::FLAG_TEST;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Robot,
    language: |_| tree_sitter_robot::LANGUAGE.into(),
    query: include_str!("../queries/robot.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |definition, _| {
            if definition.node_kind == "test_case_definition" {
                FLAG_TEST
            } else {
                0
            }
        },
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin: |name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "log"
                    | "should be equal"
                    | "should be true"
                    | "should contain"
                    | "set variable"
                    | "sleep"
                    | "run keyword if"
                    | "should not be empty"
                    | "fail"
                    | "log to console"
                    | "evaluate"
            )
        },
    },
};
