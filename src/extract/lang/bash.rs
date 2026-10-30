//! Bash: functions, commands as calls, `source` and `.` imports with literal paths, reads of
//! variables the file never assigns as environment reads.

use super::Pack;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    file_type: FileType::Bash,
    suffixes: &[],
    language: |_| tree_sitter_bash::LANGUAGE.into(),
    query: include_str!("../queries/bash.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |_: &Definition, _: &FileContext| 0,
        expand_import,
        builtin,
    },
};

/// `source` and `.` with literal paths only (section 6); expansions are not resolvable.
fn expand_import(path: &str, line: u32, reexport: bool) -> Vec<crate::domain::facts::Import> {
    if path.contains('$') || path.contains('`') {
        return Vec::new();
    }
    engine::single_import(path, line, reexport)
}

/// Shell builtins and ubiquitous utilities: calls to them are no graph edges.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "echo"
            | "printf"
            | "cd"
            | "exit"
            | "return"
            | "set"
            | "unset"
            | "export"
            | "local"
            | "read"
            | "shift"
            | "test"
            | "["
            | "[["
            | "true"
            | "false"
            | "eval"
            | "exec"
            | "trap"
            | "source"
            | "."
            | "command"
            | "type"
            | "cat"
            | "grep"
            | "sed"
            | "awk"
            | "cut"
            | "tr"
            | "sort"
            | "uniq"
            | "head"
            | "tail"
            | "wc"
            | "find"
            | "xargs"
            | "mkdir"
            | "rm"
            | "cp"
            | "mv"
            | "ln"
            | "chmod"
            | "ls"
            | "pwd"
            | "dirname"
            | "basename"
            | "date"
            | "sleep"
            | "env"
            | "wait"
            | "kill"
            | "readonly"
            | "declare"
            | "printenv"
            | "tee"
            | "touch"
            | "mktemp"
            | "realpath"
    )
}
