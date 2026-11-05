//! Solidity: `private` and `internal` members are not exported; Foundry `test*` functions in
//! `*.t.sol` files are tests.

use super::Pack;
use crate::domain::facts::FLAG_TEST;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Solidity,
    language: |_| tree_sitter_solidity::LANGUAGE.into(),
    query: include_str!("../queries/solidity.scm"),
    hooks: Hooks {
        exported: |definition| {
            let header = definition.text.split('{').next().unwrap_or_default();
            !header
                .split_whitespace()
                .any(|word| matches!(word, "private" | "internal"))
        },
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin: |name| {
            matches!(
                name,
                "require"
                    | "assert"
                    | "revert"
                    | "keccak256"
                    | "abi"
                    | "address"
                    | "payable"
                    | "uint256"
                    | "uint"
                    | "int256"
                    | "bytes32"
                    | "bytes"
                    | "string"
                    | "type"
                    | "sha256"
                    | "ecrecover"
                    | "selfdestruct"
                    | "blockhash"
                    | "gasleft"
            )
        },
    },
};

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    if context.path.ends_with(".t.sol") && definition.name.starts_with("test") {
        FLAG_TEST
    } else {
        0
    }
}
