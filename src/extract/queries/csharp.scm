; C# facts for the extraction engine (src/extract/engine.rs capture convention).

(namespace_declaration name: (_) @package)
(file_scoped_namespace_declaration name: (_) @package)

(class_declaration name: (identifier) @name) @definition.class
(record_declaration name: (identifier) @name) @definition.class
(struct_declaration name: (identifier) @name) @definition.struct
(interface_declaration name: (identifier) @name) @definition.interface
(enum_declaration name: (identifier) @name) @definition.enum
(delegate_declaration name: (identifier) @name) @definition.type
(enum_member_declaration name: (identifier) @name) @definition.constant
(method_declaration name: (identifier) @name) @definition.method
(constructor_declaration name: (identifier) @name) @definition.method
(field_declaration
  (modifier) @_const
  (variable_declaration (variable_declarator name: (identifier) @name))
  (#eq? @_const "const")) @definition.constant
(field_declaration
  (modifier) @_static
  (modifier) @_readonly
  (variable_declaration (variable_declarator name: (identifier) @name))
  (#eq? @_static "static")
  (#eq? @_readonly "readonly")) @definition.constant
(method_declaration returns: (_) @return.type) @return

(using_directive) @import.tree @import

; Base lists name a base class and interfaces alike; `I`-prefixed names are interfaces.
(base_list (identifier) @name (#match? @name "^I[A-Z]")) @reference.implement
(base_list (identifier) @name (#not-match? @name "^I[A-Z]")) @reference.inherit
(base_list (generic_name (identifier) @name) (#match? @name "^I[A-Z]")) @reference.implement
(base_list (generic_name (identifier) @name) (#not-match? @name "^I[A-Z]")) @reference.inherit
(base_list (qualified_name name: (identifier) @name) (#match? @name "^I[A-Z]")) @reference.implement

(invocation_expression function: (identifier) @name) @reference.call
(invocation_expression function: (generic_name (identifier) @name)) @reference.call
(invocation_expression
  function: (member_access_expression expression: (_) @qualifier name: (identifier) @name)) @reference.call
(invocation_expression
  function: (member_access_expression expression: (_) @qualifier name: (generic_name (identifier) @name))) @reference.call
(object_creation_expression type: (identifier) @name) @reference.call
(object_creation_expression type: (generic_name (identifier) @name)) @reference.call
(object_creation_expression type: (qualified_name qualifier: (_) @qualifier name: (identifier) @name)) @reference.call

(variable_declaration type: (identifier) @name) @reference.type
(variable_declaration type: (generic_name (identifier) @name)) @reference.type
(parameter type: (identifier) @name) @reference.type
(parameter type: (generic_name (identifier) @name)) @reference.type
(method_declaration returns: (identifier) @name) @reference.type
(method_declaration returns: (generic_name (identifier) @name)) @reference.type
(property_declaration type: (identifier) @name) @reference.type
(property_declaration type: (generic_name (identifier) @name)) @reference.type
(type_argument_list (identifier) @name) @reference.type
(typeof_expression type: (identifier) @name) @reference.type
(qualified_name qualifier: (identifier) @qualifier name: (identifier) @name) @reference.type
(attribute name: (identifier) @name) @reference.type

(parameter type: (_) @binding.type name: (identifier) @binding.name) @binding
(local_declaration_statement
  (variable_declaration
    type: (_) @binding.type
    (variable_declarator name: (identifier) @binding.name))
  (#not-eq? @binding.type "var")) @binding
(local_declaration_statement
  (variable_declaration
    type: (implicit_type)
    (variable_declarator
      name: (identifier) @binding.name
      (object_creation_expression type: (_) @binding.type)))) @binding
(local_declaration_statement
  (variable_declaration
    type: (implicit_type)
    (variable_declarator
      name: (identifier) @binding.name
      (invocation_expression function: (identifier) @binding.call)))) @binding
(foreach_statement type: (_) @binding.type left: (identifier) @binding.name) @binding

(field_declaration
  (variable_declaration
    type: (_) @field.type
    (variable_declarator name: (identifier) @field.name))) @field
(property_declaration type: (_) @field.type name: (identifier) @field.name) @field

(invocation_expression
  function: (member_access_expression expression: (identifier) @_environment name: (identifier) @_get)
  arguments: (argument_list . (argument (string_literal) @env.name))
  (#eq? @_environment "Environment")
  (#eq? @_get "GetEnvironmentVariable")) @env

; Identifiers used as values: arguments, returns, initializers, comparisons, `Type.MEMBER`.
(argument (identifier) @name @reference.value)
(return_statement (identifier) @name @reference.value)
(variable_declarator (identifier) @name @reference.value .)
(binary_expression [left: (identifier) right: (identifier)] @name @reference.value)
(member_access_expression
  expression: (identifier) @qualifier
  name: (identifier) @name
  (#match? @qualifier "^[A-Z]")) @reference.value
