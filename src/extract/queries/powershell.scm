; PowerShell facts for the extraction engine (src/extract/engine.rs capture convention).

(function_statement (function_name) @name) @definition.function
(class_statement . (simple_name) @name) @definition.class
(class_statement . (simple_name) . (simple_name) @name) @reference.inherit
(class_method_definition (simple_name) @name) @definition.method

(command
  command_name: (command_name) @_import
  command_elements: (command_elements . (command_argument_sep)? . (generic_token) @import.path)
  (#match? @_import "^(?i)Import-Module$")) @import
(command
  command_name: (command_name) @_using
  command_elements: (command_elements (generic_token) @_module (generic_token) @import.path)
  (#match? @_using "^(?i)using$")
  (#match? @_module "^(?i)module$")) @import
; Dot-sourcing and script invocation: `. ./helpers.ps1`, `& ./run.ps1`.
(command (command_invokation_operator) command_name: (command_name_expr) @import.path) @import

(command command_name: (command_name) @name) @reference.call
(invokation_expression (variable) @qualifier (member_name (simple_name) @name)) @reference.call
(invokation_expression (type_literal (type_spec (type_name (type_identifier) @qualifier))) (member_name (simple_name) @name)) @reference.call
(invokation_expression
  (type_literal (type_spec (type_name (type_identifier) @name)))
  (member_name (simple_name) @_new)
  (#match? @_new "^(?i)new$")) @reference.call
(type_literal (type_spec (type_name (type_identifier) @name))) @reference.type

((variable) @env.name @env (#match? @env.name "^\\$(?i)env:"))
