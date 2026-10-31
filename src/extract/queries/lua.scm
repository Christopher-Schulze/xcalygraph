; Lua facts for the extraction engine (src/extract/engine.rs capture convention).

(function_declaration name: (identifier) @name) @definition.function
(function_declaration name: (dot_index_expression field: (identifier) @name)) @definition.function
(function_declaration name: (method_index_expression method: (identifier) @name)) @definition.function

(variable_declaration
  (assignment_statement
    (variable_list name: (identifier) @import.alias)
    (expression_list
      value: (function_call
        name: (identifier) @_require
        arguments: (arguments (string content: (string_content) @import.path)))))
  (#eq? @_require "require")) @import
([(chunk) (block)]
  (function_call
    name: (identifier) @_require
    arguments: (arguments (string content: (string_content) @import.path))
    (#eq? @_require "require")) @import)

; `local assert_arg = utils.assert_arg`: a member of a required module re-bound locally.
(variable_declaration
  (assignment_statement
    (variable_list name: (identifier) @import.alias)
    (expression_list value: (dot_index_expression table: (identifier) @import.path field: (identifier) @import.name)))) @import

(function_call name: (identifier) @name) @reference.call
(function_call name: (dot_index_expression table: (_) @qualifier field: (identifier) @name)) @reference.call
(function_call name: (method_index_expression table: (_) @qualifier method: (identifier) @name)) @reference.call

(function_call
  name: (dot_index_expression table: (identifier) @_os field: (identifier) @_getenv)
  arguments: (arguments . (string content: (string_content) @env.name))
  (#eq? @_os "os")
  (#eq? @_getenv "getenv")) @env

; Identifiers used as values: arguments.
(arguments (identifier) @name @reference.value)
