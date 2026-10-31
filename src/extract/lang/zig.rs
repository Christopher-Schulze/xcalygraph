//! Zig: `pub` exported, `test` blocks as tests, `main` entry, `@import` with its binding name.

use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Zig,
    language: |_| tree_sitter_zig::LANGUAGE.into(),
    query: include_str!("../queries/zig.scm"),
    hooks: Hooks {
        exported: |definition| definition.text.trim_start().starts_with("pub"),
        signature_prefix: |_, _| "",
        flags,
        expand_import: engine::single_import,
        builtin: |name| matches!(name, "std" | "@import" | "print"),
    },
};

fn flags(definition: &Definition, _context: &FileContext) -> u8 {
    if definition.text.trim_start().starts_with("test") {
        FLAG_TEST
    } else if definition.name == "main" {
        FLAG_ENTRY
    } else {
        0
    }
}
