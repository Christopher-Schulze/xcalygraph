//! C++ (also CUDA `.cu`, `.cuh` and Metal `.metal`): C hooks, plus `std` names as builtins.

use super::Pack;
use super::c::{exported, flags, include_import, signature_prefix};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Cpp,
    language: |_| tree_sitter_cpp::LANGUAGE.into(),
    query: include_str!("../queries/cpp.scm"),
    hooks: Hooks {
        exported,
        signature_prefix,
        flags,
        expand_import: ImportExpansion::Text(include_import),
        builtin,
    },
};

/// C library and C++ standard library names (written without `std::` after `using namespace
/// std`): references to them carry no project edge.
fn builtin(name: &str) -> bool {
    super::c::builtin(name)
        || matches!(
            name,
            "std"
                | "string"
                | "string_view"
                | "vector"
                | "map"
                | "unordered_map"
                | "set"
                | "unordered_set"
                | "array"
                | "optional"
                | "variant"
                | "unique_ptr"
                | "shared_ptr"
                | "weak_ptr"
                | "make_unique"
                | "make_shared"
                | "move"
                | "forward"
                | "cout"
                | "cerr"
                | "endl"
                | "nullptr"
                | "auto"
                | "size"
                | "begin"
                | "end"
        )
}
