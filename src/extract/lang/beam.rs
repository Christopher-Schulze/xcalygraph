//! Elixir and Erlang: `defp` private, ExUnit and Common Test modules in test files, OTP
//! callbacks and Phoenix controller actions as entrypoints.

use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks};

/// Callbacks a behaviour (GenServer, Supervisor, Application, gen_server) calls.
fn callback(name: &str) -> bool {
    matches!(
        name,
        "init"
            | "handle_call"
            | "handle_cast"
            | "handle_info"
            | "handle_continue"
            | "terminate"
            | "code_change"
            | "start"
            | "start_link"
            | "child_spec"
            | "stop"
    )
}

#[cfg(feature = "lang-elixir")]
pub static ELIXIR: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Elixir,
    language: |_| tree_sitter_elixir::LANGUAGE.into(),
    query: include_str!("../queries/elixir.scm"),
    hooks: Hooks {
        exported: |definition| !definition.text.trim_start().starts_with("defp"),
        signature_prefix: |_, _| "",
        flags: elixir_flags,
        expand_import: engine::single_import,
        builtin: elixir_builtin,
    },
};

#[cfg(feature = "lang-elixir")]
fn elixir_flags(definition: &Definition, context: &FileContext) -> u8 {
    let function = matches!(definition.kind, SymKind::Fn | SymKind::Method);
    let framework =
        context.path.contains("_web/controllers/") || context.path.contains("_web/live/");
    if function && (callback(definition.name) || framework) {
        FLAG_ENTRY
    } else {
        0
    }
}

#[cfg(feature = "lang-elixir")]
fn elixir_builtin(name: &str) -> bool {
    matches!(
        name,
        "def"
            | "defp"
            | "defmodule"
            | "defmacro"
            | "defmacrop"
            | "defprotocol"
            | "defimpl"
            | "defstruct"
            | "defexception"
            | "defguard"
            | "defdelegate"
            | "alias"
            | "import"
            | "use"
            | "require"
            | "if"
            | "unless"
            | "case"
            | "cond"
            | "with"
            | "for"
            | "fn"
            | "raise"
            | "quote"
            | "unquote"
            | "test"
            | "describe"
            | "setup"
            | "assert"
            | "refute"
            | "is_nil"
            | "is_binary"
            | "is_integer"
            | "is_map"
            | "is_list"
            | "is_atom"
            | "to_string"
            | "inspect"
            | "elem"
            | "map_size"
            | "moduledoc"
            | "doc"
            | "spec"
            | "impl"
            | "type"
            | "typep"
            | "opaque"
            | "callback"
            | "behaviour"
            | "derive"
    )
}

#[cfg(feature = "lang-erlang")]
pub static ERLANG: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Erlang,
    language: |_| tree_sitter_erlang::LANGUAGE.into(),
    query: include_str!("../queries/erlang.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: erlang_flags,
        expand_import: engine::single_import,
        builtin: |name| {
            matches!(
                name,
                "length"
                    | "element"
                    | "is_list"
                    | "is_atom"
                    | "is_integer"
                    | "self"
                    | "spawn"
                    | "throw"
                    | "error"
                    | "exit"
            )
        },
    },
};

#[cfg(feature = "lang-erlang")]
fn erlang_flags(definition: &Definition, _context: &FileContext) -> u8 {
    if callback(definition.name) || definition.name == "main" {
        FLAG_ENTRY
    } else {
        0
    }
}
