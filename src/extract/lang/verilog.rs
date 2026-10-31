//! Verilog and SystemVerilog: modules, interfaces, packages, classes, functions and tasks;
//! instantiations call the instantiated module.

use super::Pack;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Verilog,
    language: |_| tree_sitter_verilog::LANGUAGE.into(),
    query: include_str!("../queries/verilog.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |_, _| 0,
        expand_import: engine::single_import,
        builtin: |name| name.starts_with('$'),
    },
};
