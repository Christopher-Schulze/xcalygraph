; Julia facts for the extraction engine (src/extract/engine.rs capture convention).

(module_definition name: (identifier) @name) @definition.module
(function_definition (signature (call_expression . (identifier) @name))) @definition.function
(function_definition (signature (call_expression . (field_expression (identifier) @name .)))) @definition.function
; `f(x::T) where T` and `f(x)::R` signatures wrap the call.
(function_definition (signature (where_expression . (call_expression . (identifier) @name)))) @definition.function
(function_definition (signature (where_expression . (call_expression . (field_expression (identifier) @name .))))) @definition.function
(function_definition (signature (typed_expression . (call_expression . (identifier) @name)))) @definition.function
(function_definition (signature (typed_expression . (call_expression . (field_expression (identifier) @name .))))) @definition.function
(function_definition (signature (where_expression . (typed_expression . (call_expression . (identifier) @name))))) @definition.function
(function_definition (signature (where_expression . (typed_expression . (call_expression . (field_expression (identifier) @name .)))))) @definition.function
(macro_definition (signature (call_expression . (identifier) @name))) @definition.macro
; Short form `f(x) = ...` and method extensions `Base.show(io, x) = ...`.
(assignment . (call_expression . (identifier) @name) (operator)) @definition.function
(assignment . (call_expression . (field_expression (identifier) @name .)) (operator)) @definition.function
(assignment . (where_expression . (call_expression . (identifier) @name)) (operator)) @definition.function
(assignment . (where_expression . (call_expression . (field_expression (identifier) @name .))) (operator)) @definition.function
(struct_definition (type_head (identifier) @name)) @definition.struct
(struct_definition (type_head (binary_expression . (identifier) @name))) @definition.struct
(struct_definition (type_head (binary_expression (operator) (identifier) @name)) @reference.inherit)
(abstract_definition (type_head (identifier) @name)) @definition.interface
(abstract_definition (type_head (binary_expression . (identifier) @name))) @definition.interface
(const_statement (assignment . (identifier) @name)) @definition.constant

; One import per module (`using A, B`).
(using_statement [(identifier) (scoped_identifier)] @import.path @import)
(using_statement (selected_import . (_) @import.path)) @import
(import_statement [(identifier) (scoped_identifier)] @import.path @import)
(import_statement (selected_import (import_path) @import.path)) @import
(call_expression
  (identifier) @_include
  (argument_list . (string_literal (content) @import.path))
  (#eq? @_include "include")) @import

(call_expression . (identifier) @name) @reference.call
(call_expression . (field_expression value: (_) @qualifier (identifier) @name .)) @reference.call
(binary_expression (operator) @_pipe . (identifier) @name (#eq? @_pipe "|>")) @reference.call
(typed_expression (_) (identifier) @name) @reference.type
(parametrized_type_expression . (identifier) @name) @reference.type

(index_expression
  (identifier) @_env
  (vector_expression . (string_literal (content) @env.name))
  (#eq? @_env "ENV")) @env
(call_expression
  (identifier) @_get
  (argument_list . (identifier) @_env . (string_literal (content) @env.name))
  (#eq? @_get "get")
  (#eq? @_env "ENV")) @env
