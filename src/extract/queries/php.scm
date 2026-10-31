; PHP facts for the extraction engine (src/extract/engine.rs capture convention).

(namespace_definition name: (namespace_name) @package)

(class_declaration name: (name) @name) @definition.class
(interface_declaration name: (name) @name) @definition.interface
(trait_declaration name: (name) @name) @definition.trait
(enum_declaration name: (name) @name) @definition.enum
(method_declaration name: (name) @name) @definition.method
(function_definition name: (name) @name) @definition.function
(const_declaration (const_element (name) @name)) @definition.constant
(method_declaration return_type: (_) @return.type) @return

(namespace_use_declaration) @import.tree @import

(base_clause [(name) @name (qualified_name (name) @name .)]) @reference.inherit
(class_interface_clause [(name) @name (qualified_name (name) @name .)]) @reference.implement
(use_declaration [(name) @name (qualified_name (name) @name .)]) @reference.implement

(function_call_expression function: [(name) @name (qualified_name (name) @name .)]) @reference.call
(member_call_expression object: (_) @qualifier name: (name) @name) @reference.call
(nullsafe_member_call_expression object: (_) @qualifier name: (name) @name) @reference.call
(scoped_call_expression scope: (_) @qualifier name: (name) @name) @reference.call
(object_creation_expression [(name) @name (qualified_name (name) @name .)]) @reference.call
(named_type [(name) @name (qualified_name (name) @name .)]) @reference.type
(class_constant_access_expression . [(name) @name (qualified_name (name) @name .)]) @reference.type

(simple_parameter type: (_) @binding.type name: (variable_name) @binding.name) @binding
(assignment_expression
  left: (variable_name) @binding.name
  right: (object_creation_expression [(name) (qualified_name)] @binding.ctor)) @binding
(property_declaration type: (_) @field.type (property_element (variable_name (name) @field.name))) @field
; Constructor property promotion (`public function __construct(private Repo $repo)`).
(property_promotion_parameter type: (_) @field.type name: (variable_name (name) @field.name)) @field

(function_call_expression
  function: (name) @_getenv
  arguments: (arguments . (argument (string (string_content) @env.name)))
  (#eq? @_getenv "getenv")) @env
(subscript_expression
  (variable_name (name) @_env)
  (string (string_content) @env.name)
  (#any-of? @_env "_ENV" "_SERVER")) @env

; Identifiers used as values: arguments.
(argument (variable_name) @name @reference.value)
