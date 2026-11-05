//! Lua: `local function` is private, `require` paths as imports; `.toc` file lists.

use super::Pack;
use crate::domain::facts::{Facts, Import};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Lua,
    language: |_| tree_sitter_lua::LANGUAGE.into(),
    query: include_str!("../queries/lua.scm"),
    hooks: Hooks {
        exported: |definition| !definition.text.trim_start().starts_with("local"),
        signature_prefix: |_, _| "",
        flags: |_, _| 0,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin: |name| {
            matches!(
                name,
                "print"
                    | "require"
                    | "pairs"
                    | "ipairs"
                    | "type"
                    | "tostring"
                    | "tonumber"
                    | "error"
                    | "assert"
                    | "pcall"
                    | "select"
                    | "setmetatable"
                    | "getmetatable"
                    | "rawget"
                    | "rawset"
                    | "unpack"
                    | "next"
            )
        },
    },
};

/// Files listed in a `.toc` addon manifest (one path per line, `#` comments and `##` metadata
/// skipped), as imports relative to the manifest.
pub fn toc_imports(text: &str, facts: &mut Facts) {
    for (index, line) in text.lines().enumerate() {
        let path = line.trim();
        if path.is_empty() || path.starts_with('#') {
            continue;
        }
        facts.imports.push(Import {
            spec: path.replace('\\', "/"),
            line: index as u32 + 1,
            ..Import::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_lists_files_without_metadata() {
        let mut facts = Facts::default();
        toc_imports(
            "## Interface: 110000\n## Title: Store\n\n# core\nLibs\\LibStub\\LibStub.lua\nCore.lua\nUI.xml\n",
            &mut facts,
        );
        let specs: Vec<(u32, &str)> = facts
            .imports
            .iter()
            .map(|import| (import.line, import.spec.as_str()))
            .collect();
        assert_eq!(
            specs,
            [
                (5, "Libs/LibStub/LibStub.lua"),
                (6, "Core.lua"),
                (7, "UI.xml")
            ]
        );
    }
}
