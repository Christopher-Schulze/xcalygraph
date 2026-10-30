; TypeScript facts for the extraction engine (src/extract/engine.rs capture convention).

(program (lexical_declaration (variable_declarator name: (identifier) @name) @definition.constant))
(program (export_statement (lexical_declaration (variable_declarator name: (identifier) @name) @definition.constant)))
(function_declaration name: (identifier) @name) @definition.function
(function_declaration return_type: (type_annotation (_) @return.type)) @return
(generator_function_declaration name: (identifier) @name) @definition.function
(function_signature name: (identifier) @name) @definition.function
(program (lexical_declaration (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @definition.function))
(program (export_statement (lexical_declaration (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @definition.function)))
(class_declaration name: (type_identifier) @name) @definition.class
(abstract_class_declaration name: (type_identifier) @name) @definition.class
(method_definition name: (property_identifier) @name) @definition.method
(method_definition return_type: (type_annotation (_) @return.type)) @return
(method_signature name: (property_identifier) @name) @definition.method
(abstract_method_signature name: (property_identifier) @name) @definition.method
(interface_declaration name: (type_identifier) @name) @definition.interface
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.enum

(class_heritage (extends_clause value: (identifier) @name) @reference.inherit)
(class_heritage (extends_clause value: (member_expression object: (_) @qualifier property: (property_identifier) @name)) @reference.inherit)
(implements_clause (type_identifier) @name @reference.implementation)
(implements_clause (generic_type name: (type_identifier) @name) @reference.implementation)
(extends_type_clause (type_identifier) @name @reference.inherit)

(import_statement) @import.tree @import
(export_statement source: (string)) @import.tree @import
(variable_declarator
  name: (identifier) @import.alias
  value: (call_expression function: (identifier) @_require arguments: (arguments . (string) @import.path))
  (#eq? @_require "require")) @import

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (member_expression object: (_) @qualifier property: (property_identifier) @name)) @reference.call
(new_expression constructor: (identifier) @name) @reference.call
(new_expression constructor: (member_expression object: (_) @qualifier property: (property_identifier) @name)) @reference.call
(type_identifier) @name @reference.type
(nested_type_identifier module: (_) @qualifier name: (type_identifier) @name) @reference.type

(required_parameter pattern: (identifier) @binding.name type: (type_annotation (_) @binding.type)) @binding
(optional_parameter pattern: (identifier) @binding.name type: (type_annotation (_) @binding.type)) @binding
(variable_declarator name: (identifier) @binding.name type: (type_annotation (_) @binding.type)) @binding
(variable_declarator name: (identifier) @binding.name value: (new_expression constructor: (_) @binding.ctor)) @binding
(variable_declarator name: (identifier) @binding.name value: (call_expression function: (_) @binding.call)) @binding
(variable_declarator name: (identifier) @binding.name value: (await_expression (call_expression function: (_) @binding.call))) @binding

(public_field_definition name: (property_identifier) @field.name type: (type_annotation (_) @field.type)) @field
(public_field_definition name: (property_identifier) @field.name value: (new_expression constructor: (_) @field.type)) @field
(required_parameter (accessibility_modifier) pattern: (identifier) @field.name type: (type_annotation (_) @field.type)) @field
(assignment_expression
  left: (member_expression object: (this) property: (property_identifier) @field.name)
  right: (new_expression constructor: (_) @field.type)) @field

(member_expression
  object: (member_expression object: (identifier) @_process property: (property_identifier) @_env)
  property: (property_identifier) @env.name
  (#eq? @_process "process")
  (#eq? @_env "env")) @env
(subscript_expression
  object: (member_expression object: (identifier) @_process property: (property_identifier) @_env)
  index: (string) @env.name
  (#eq? @_process "process")
  (#eq? @_env "env")) @env
