; Erlang facts for the extraction engine (src/extract/engine.rs capture convention).

(module_attribute name: (atom) @package)
; Each clause is a `fun_decl`; the engine merges consecutive clauses into one function.
(fun_decl clause: (function_clause name: (atom) @name)) @definition.function
(record_decl name: (atom) @name) @definition.struct

(pp_include file: (_) @import.path) @import
(import_attribute module: (atom) @import.path) @import
(behaviour_attribute name: (atom) @name) @reference.implement

(call expr: (atom) @name) @reference.call
(remote module: (remote_module module: (atom) @qualifier) fun: (call expr: (atom) @name) @reference.call)

(remote
  module: (remote_module module: (atom) @_os)
  fun: (call expr: (atom) @_getenv args: (expr_args . (string) @env.name))
  (#eq? @_os "os")
  (#eq? @_getenv "getenv")) @env
