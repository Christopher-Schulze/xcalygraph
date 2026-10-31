; Visual Basic .NET facts for the extraction engine (src/extract/engine.rs capture convention).
; Imports, Inherits and Implements lines are scanned separately (src/extract/lang/vbnet.rs):
; the grammar misparses aliases and supertype lines.

(namespace_block name: (namespace_name) @package)

(class_block name: (identifier) @name) @definition.class
(module_block name: (identifier) @name) @definition.class
(interface_block name: (identifier) @name) @definition.interface
(structure_block name: (identifier) @name) @definition.struct
(enum_block name: (identifier) @name) @definition.enum
(enum_member name: (identifier) @name) @definition.constant
(method_declaration name: (identifier) @name) @definition.method
(const_declaration name: (identifier) @name) @definition.constant
(method_declaration return_type: (type) @return.type) @return

(invocation target: (identifier) @name) @reference.call
(invocation target: (member_access object: (_) @qualifier member: (identifier) @name)) @reference.call
(new_expression type: (type (namespace_name (identifier) @name .))) @reference.call
(as_clause type: (type (namespace_name (identifier) @name .))) @reference.type
(attribute name: (identifier) @name) @reference.type

(parameter name: (identifier) @binding.name (as_clause type: (type) @binding.type)) @binding
(dim_statement name: (identifier) @binding.name (as_clause type: (type) @binding.type)) @binding
; Without `As` clause (anchored: the initializer follows the name) the constructed type.
(dim_statement
  name: (identifier) @binding.name
  .
  initializer: (expression (new_expression type: (type) @binding.type))) @binding

(field_declaration (variable_declarator name: (identifier) @field.name (as_clause type: (type) @field.type))) @field
(property_declaration name: (identifier) @field.name (as_clause type: (type) @field.type)) @field

(invocation
  target: (member_access object: (expression (identifier) @_environment) member: (identifier) @_get)
  arguments: (argument_list . (argument (expression (literal (string_literal) @env.name))))
  (#eq? @_environment "Environment")
  (#eq? @_get "GetEnvironmentVariable")) @env

; Identifiers used as values: arguments.
(argument (expression (identifier) @name @reference.value))
