; Elixir facts for the extraction engine (src/extract/engine.rs capture convention). Modules are
; named by their full alias (`Store.Repo`).

(call target: (identifier) @_def (arguments (alias) @name) (#eq? @_def "defmodule")) @definition.class
(call target: (identifier) @_def (arguments (alias) @name) (#eq? @_def "defprotocol")) @definition.interface
(call
  target: (identifier) @_def
  (arguments
    [(identifier) @name
     (call target: (identifier) @name)
     (binary_operator left: (call target: (identifier) @name) operator: "when")])
  (#any-of? @_def "def" "defp" "defmacro" "defmacrop" "defguard" "defguardp" "defdelegate")) @definition.function

(call target: (identifier) @_impl (arguments (alias) @name) (#eq? @_impl "defimpl")) @reference.implement
; `alias` binds the last segment (marked by the alias capture); `import` and `use` make the
; module's functions visible.
(call
  target: (identifier) @_alias
  (arguments . (alias) @import.path @import.alias)
  (#eq? @_alias "alias")) @import
(call
  target: (identifier) @_import
  (arguments . (alias) @import.path)
  (#any-of? @_import "import" "use" "require")) @import

(call target: (identifier) @name) @reference.call
; `Mod.fun` is a call with or without parentheses; `var.key` without arguments is a map field.
(call target: (dot left: (alias) @qualifier right: (identifier) @name)) @reference.call
(call target: (dot left: (_) @qualifier right: (identifier) @name) (arguments)) @reference.call
(binary_operator operator: "|>" right: (identifier) @name) @reference.call
(alias) @name @reference.type

(call
  target: (dot left: (alias) @_system right: (identifier) @_get)
  (arguments . (string (quoted_content) @env.name))
  (#eq? @_system "System")
  (#any-of? @_get "get_env" "fetch_env" "fetch_env!")) @env
