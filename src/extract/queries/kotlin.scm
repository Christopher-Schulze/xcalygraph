; Kotlin facts for the extraction engine (src/extract/engine.rs capture convention).

(package_header (qualified_identifier) @package)

(class_declaration name: (identifier) @name) @definition.class
(class_declaration "interface" name: (identifier) @name) @definition.interface
(class_declaration
  (modifiers (class_modifier) @_enum)
  name: (identifier) @name
  (#eq? @_enum "enum")) @definition.enum
(object_declaration name: (identifier) @name) @definition.class
(function_declaration name: (identifier) @name) @definition.function
(function_declaration (function_value_parameters) . (user_type) @return.type) @return
(source_file (property_declaration (variable_declaration (identifier) @name)) @definition.variable)
(property_declaration
  (modifiers (property_modifier) @_const)
  (variable_declaration (identifier) @name)
  (#eq? @_const "const")) @definition.constant
(type_alias type: (identifier) @name) @definition.type
(enum_entry (identifier) @name) @definition.constant

(import) @import.tree @import

(delegation_specifier (constructor_invocation (user_type . (identifier) @name))) @reference.inherit
(delegation_specifier (user_type . (identifier) @name)) @reference.implement
; `Taggable by tagMap`: the class implements the interface through a delegate.
(delegation_specifier (explicit_delegation (user_type . (identifier) @name))) @reference.implement

(call_expression . (identifier) @name (value_arguments)) @reference.call
(call_expression . (identifier) @name (annotated_lambda)) @reference.call
; `!f(x)` parses as a call of the negation.
(call_expression . (unary_expression (identifier) @name) (value_arguments)) @reference.call
(call_expression
  . (navigation_expression . (_) @qualifier (identifier) @name .)
  [(value_arguments) (annotated_lambda)]) @reference.call
(callable_reference (identifier) @name .) @reference.call
(user_type . (identifier) @name) @reference.type
(annotation (user_type . (identifier) @name)) @reference.type
(annotation (constructor_invocation (user_type . (identifier) @name))) @reference.type

(parameter (identifier) @binding.name (user_type) @binding.type) @binding
(class_parameter (identifier) @binding.name (user_type) @binding.type) @binding
(property_declaration (variable_declaration (identifier) @binding.name (user_type) @binding.type)) @binding
(property_declaration
  (variable_declaration (identifier) @binding.name)
  (call_expression . (identifier) @binding.ctor)
  (#match? @binding.ctor "^[A-Z]")) @binding
(property_declaration
  (variable_declaration (identifier) @binding.name)
  (call_expression . (identifier) @binding.call)
  (#match? @binding.call "^[a-z_]")) @binding
(for_statement (variable_declaration (identifier) @binding.name (user_type) @binding.type)) @binding

(class_body
  (property_declaration (variable_declaration (identifier) @field.name (user_type) @field.type)) @field)
(primary_constructor
  (class_parameters (class_parameter (identifier) @field.name (user_type) @field.type) @field))

(call_expression
  . (navigation_expression . (identifier) @_system (identifier) @_getenv .)
  (value_arguments . (value_argument (string_literal) @env.name))
  (#eq? @_system "System")
  (#eq? @_getenv "getenv")) @env

; Identifiers used as values: arguments, returns, initializers.
(value_argument (identifier) @name @reference.value)
(value_argument (navigation_expression . (identifier) @qualifier (identifier) @name .) @reference.value)
(property_declaration (variable_declaration) . (identifier) @name @reference.value)
(binary_expression [left: (identifier) right: (identifier)] @name @reference.value)
(navigation_expression . (identifier) @qualifier (identifier) @name . (#match? @qualifier "^[A-Z]")) @reference.value
