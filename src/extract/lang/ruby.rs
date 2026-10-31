//! Ruby: public by default, Minitest `test_*` methods in test files, Rails controller actions,
//! jobs and mailers as framework entrypoints; `require` paths as imports.

use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Ruby,
    language: |_| tree_sitter_ruby::LANGUAGE.into(),
    query: include_str!("../queries/ruby.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags,
        expand_import: engine::single_import,
        builtin,
    },
};

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    let method = definition.kind == SymKind::Method;
    let mut flags = 0;
    if method
        && definition.name.starts_with("test_")
        && crate::domain::role::is_test_path(FileType::Ruby, context.path)
    {
        flags |= FLAG_TEST;
    }
    // Rails calls controller actions, job `perform` and mailer methods by convention.
    let framework = [
        "app/controllers/",
        "app/jobs/",
        "app/mailers/",
        "app/channels/",
    ]
    .iter()
    .any(|dir| context.path.contains(dir));
    if method && framework {
        flags |= FLAG_ENTRY;
    }
    flags
}

/// Kernel methods and core classes: references to them carry no project edge.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "puts"
            | "print"
            | "p"
            | "pp"
            | "require"
            | "require_relative"
            | "load"
            | "raise"
            | "attr_reader"
            | "attr_writer"
            | "attr_accessor"
            | "include"
            | "extend"
            | "prepend"
            | "private"
            | "protected"
            | "public"
            | "lambda"
            | "proc"
            | "loop"
            | "block_given?"
            | "String"
            | "Integer"
            | "Float"
            | "Array"
            | "Hash"
            | "Symbol"
            | "Object"
            | "StandardError"
            | "RuntimeError"
            | "ArgumentError"
            | "NotImplementedError"
            | "Struct"
            | "Kernel"
            | "ENV"
            | "nil?"
            | "is_a?"
            | "respond_to?"
            | "freeze"
            | "frozen?"
            | "new"
    )
}
