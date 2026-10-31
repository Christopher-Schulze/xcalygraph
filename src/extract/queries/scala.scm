; Scala facts for the extraction engine (src/extract/engine.rs capture convention).

(package_clause name: (package_identifier) @package)

(class_definition name: (identifier) @name) @definition.class
(object_definition name: (identifier) @name) @definition.class
(trait_definition name: (identifier) @name) @definition.interface
(enum_definition name: (identifier) @name) @definition.enum
(simple_enum_case name: (identifier) @name) @definition.constant
(full_enum_case name: (identifier) @name) @definition.class
(function_definition name: (identifier) @name) @definition.function
(function_declaration name: (identifier) @name) @definition.function
(function_definition return_type: (_) @return.type) @return
(function_declaration return_type: (_) @return.type) @return
(type_definition name: (type_identifier) @name) @definition.type
(compilation_unit (val_definition pattern: (identifier) @name) @definition.variable)
(package_clause body: (template_body (val_definition pattern: (identifier) @name) @definition.variable))
(object_definition body: (template_body (val_definition pattern: (identifier) @name) @definition.constant))

(import_declaration) @import.tree @import

; Repeated `type:` fields match only their first child by field name: match positionally.
(extends_clause (type_identifier) @name) @reference.inherit
(extends_clause (generic_type . (type_identifier) @name)) @reference.inherit

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression value: (_) @qualifier field: (identifier) @name)) @reference.call
(call_expression function: (generic_function function: (identifier) @name)) @reference.call
(instance_expression (type_identifier) @name) @reference.call
(instance_expression (generic_type type: (type_identifier) @name)) @reference.call
(type_identifier) @name @reference.type
(stable_type_identifier (_) @qualifier . (type_identifier) @name .) @reference.type
(annotation name: (type_identifier) @name) @reference.type

(parameter name: (identifier) @binding.name type: (_) @binding.type) @binding
(class_parameter name: (identifier) @binding.name type: (_) @binding.type) @binding
(val_definition pattern: (identifier) @binding.name type: (_) @binding.type) @binding
(val_definition
  pattern: (identifier) @binding.name
  value: (instance_expression [(type_identifier) (generic_type)] @binding.type)) @binding
(val_definition
  pattern: (identifier) @binding.name
  value: (call_expression function: (identifier) @binding.ctor)
  (#match? @binding.ctor "^[A-Z]")) @binding
(val_definition
  pattern: (identifier) @binding.name
  value: (call_expression function: (identifier) @binding.call)
  (#match? @binding.call "^[a-z_]")) @binding

(class_parameter name: (identifier) @field.name type: (_) @field.type) @field
(template_body (val_definition pattern: (identifier) @field.name type: (_) @field.type) @field)
(template_body (var_definition pattern: (identifier) @field.name type: (_) @field.type) @field)

(call_expression
  function: (field_expression value: (identifier) @_system field: (identifier) @_getenv)
  arguments: (arguments . (string) @env.name)
  (#eq? @_system "System")
  (#eq? @_getenv "getenv")) @env
(call_expression
  function: (field_expression value: (identifier) @_sys field: (identifier) @_env)
  arguments: (arguments . (string) @env.name)
  (#eq? @_sys "sys")
  (#eq? @_env "env")) @env

; Identifiers used as values: arguments, initializers, `Type.MEMBER`.
(arguments (identifier) @name @reference.value)
(val_definition value: (identifier) @name @reference.value)
; Objects are often lowercase (`read`, `_home`): every `a.b` may name an object member.
(field_expression value: (identifier) @qualifier field: (identifier) @name) @reference.value
