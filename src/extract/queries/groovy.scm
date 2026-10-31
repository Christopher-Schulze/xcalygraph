; Groovy facts for the extraction engine (Java-derived grammar) (src/extract/engine.rs capture convention).

(package_declaration [(scoped_identifier) (identifier)] @package)

(class_declaration name: (identifier) @name) @definition.class
(interface_declaration name: (identifier) @name) @definition.interface
(annotation_type_declaration name: (identifier) @name) @definition.interface
(enum_declaration name: (identifier) @name) @definition.enum
(record_declaration name: (identifier) @name) @definition.class
(method_declaration name: (identifier) @name (#not-eq? @name "def")) @definition.method
; Spock feature methods (`def "does x"()`) misparse with `def` as name: their calls belong to
; the specification class.
(function_definition name: (identifier) @name (#not-eq? @name "def")) @definition.function
(function_definition type: (_) @return.type) @return
(constructor_declaration name: (identifier) @name (#not-eq? @name "def")) @definition.method
(field_declaration
  (modifiers) @_modifiers
  declarator: (variable_declarator name: (identifier) @name)
  (#match? @_modifiers "static")
  (#match? @_modifiers "final")) @definition.constant
(enum_constant name: (identifier) @name) @definition.constant
(method_declaration type: (_) @return.type) @return

(import_declaration) @import.tree @import

(superclass (type_identifier) @name) @reference.inherit
(superclass (scoped_type_identifier (_) @qualifier . (type_identifier) @name .)) @reference.inherit
(super_interfaces (type_list (scoped_type_identifier (_) @qualifier . (type_identifier) @name .))) @reference.implement
(super_interfaces (type_list (generic_type (scoped_type_identifier (_) @qualifier . (type_identifier) @name .)))) @reference.implement
(superclass (generic_type (type_identifier) @name)) @reference.inherit
(super_interfaces (type_list (type_identifier) @name)) @reference.implement
(super_interfaces (type_list (generic_type (type_identifier) @name))) @reference.implement
(extends_interfaces (type_list (type_identifier) @name)) @reference.inherit
(extends_interfaces (type_list (generic_type (type_identifier) @name))) @reference.inherit

(method_invocation !object name: (identifier) @name) @reference.call
(juxt_function_call name: (identifier) @name) @reference.call
(method_invocation object: (_) @qualifier name: (identifier) @name) @reference.call
(object_creation_expression type: (type_identifier) @name) @reference.call
(object_creation_expression type: (generic_type (type_identifier) @name)) @reference.call
(method_reference (_) @qualifier (identifier) @name .) @reference.call
(type_identifier) @name @reference.type
(marker_annotation name: (identifier) @name) @reference.type
(annotation name: (identifier) @name) @reference.type
(scoped_type_identifier (_) @qualifier . (type_identifier) @name .) @reference.type

(formal_parameter type: (_) @binding.type name: (identifier) @binding.name) @binding
(local_variable_declaration
  type: (_) @binding.type
  declarator: (variable_declarator name: (identifier) @binding.name)
  (#not-any-of? @binding.type "var" "def")) @binding
(local_variable_declaration
  type: (type_identifier) @_var
  declarator: (variable_declarator
    name: (identifier) @binding.name
    value: (object_creation_expression type: (_) @binding.type))
  (#any-of? @_var "var" "def")) @binding
(local_variable_declaration
  type: (type_identifier) @_var
  declarator: (variable_declarator
    name: (identifier) @binding.name
    value: (method_invocation !object name: (identifier) @binding.call))
  (#any-of? @_var "var" "def")) @binding
(enhanced_for_statement type: (_) @binding.type name: (identifier) @binding.name) @binding

(field_declaration
  type: (_) @field.type
  declarator: (variable_declarator name: (identifier) @field.name)) @field

(method_invocation
  object: (identifier) @_system
  name: (identifier) @_getenv
  arguments: (argument_list . (string_literal) @env.name)
  (#eq? @_system "System")
  (#eq? @_getenv "getenv")) @env

; Identifiers used as values: arguments, returns, assignments, initializers.
(argument_list (identifier) @name @reference.value)
(argument_list (field_access object: (identifier) @qualifier field: (identifier) @name) @reference.value)
(return_statement (identifier) @name @reference.value)
(binary_expression [left: (identifier) right: (identifier)] @name @reference.value)
(binary_expression
  [left: (field_access object: (identifier) @qualifier field: (identifier) @name)
   right: (field_access object: (identifier) @qualifier field: (identifier) @name)] @reference.value)
(assignment_expression right: (identifier) @name @reference.value)
(variable_declarator value: (identifier) @name @reference.value)
(variable_declarator value: (field_access object: (identifier) @qualifier field: (identifier) @name) @reference.value)
(field_access object: (identifier) @qualifier field: (identifier) @name (#match? @qualifier "^[A-Z]")) @reference.value
