; Python facts for the extraction engine (src/extract/engine.rs capture convention).

(function_definition name: (identifier) @name) @definition.function
(function_definition return_type: (type) @return.type) @return
(class_definition name: (identifier) @name) @definition.class
(class_definition superclasses: (argument_list (identifier) @name) @reference.inherit)
(class_definition
  superclasses: (argument_list (attribute object: (_) @qualifier attribute: (identifier) @name)) @reference.inherit)
(module (expression_statement (assignment left: (identifier) @name)) @definition.variable)
; Annotated class attributes (dataclasses, attrs, pydantic, typed classes) are fields.
(class_definition
  body: (block (expression_statement (assignment left: (identifier) @field.name type: (type) @field.type) @field)))

(import_statement) @import.tree @import
(import_from_statement) @import.tree @import

(call function: (identifier) @name) @reference.call
(call function: (attribute object: (_) @qualifier attribute: (identifier) @name)) @reference.call
; Decorators apply a callable to the decorated definition.
(decorator (identifier) @name @reference.call)
(decorator (attribute object: (_) @qualifier attribute: (identifier) @name) @reference.call)
; Attribute reads (`user.display`): properties and methods passed as values; a value reference
; only counts when the receiver's type is known and has the member.
(attribute object: (_) @qualifier attribute: (identifier) @name) @reference.value
(type (identifier) @name) @reference.type
(type (attribute object: (_) @qualifier attribute: (identifier) @name)) @reference.type

(typed_parameter (identifier) @binding.name type: (type) @binding.type) @binding
(typed_default_parameter name: (identifier) @binding.name type: (type) @binding.type) @binding
(assignment left: (identifier) @binding.name type: (type) @binding.type) @binding
(assignment
  left: (identifier) @binding.name
  right: (call function: [(identifier) (attribute)] @binding.ctor)
  (#match? @binding.ctor "(^|[.])[A-Z][A-Za-z0-9_]*$")) @binding
(assignment
  left: (identifier) @binding.name
  right: (call function: [(identifier) (attribute)] @binding.call)
  (#match? @binding.call "(^|[.])[a-z_][A-Za-z0-9_]*$")) @binding

(assignment
  left: (attribute object: (identifier) @_self attribute: (identifier) @field.name)
  right: (call function: [(identifier) (attribute)] @field.type)
  (#eq? @_self "self")) @field
(assignment
  left: (attribute object: (identifier) @_self attribute: (identifier) @field.name)
  type: (type) @field.type
  (#eq? @_self "self")) @field

(call
  function: (attribute object: (identifier) @_os attribute: (identifier) @_fn)
  arguments: (argument_list . (string) @env.name)
  (#eq? @_os "os")
  (#eq? @_fn "getenv")) @env
(call
  function: (attribute
    object: (attribute object: (identifier) @_os attribute: (identifier) @_environ)
    attribute: (identifier) @_get)
  arguments: (argument_list . (string) @env.name)
  (#eq? @_os "os")
  (#eq? @_environ "environ")
  (#eq? @_get "get")) @env
(subscript
  value: (attribute object: (identifier) @_os attribute: (identifier) @_environ)
  subscript: (string) @env.name
  (#eq? @_os "os")
  (#eq? @_environ "environ")) @env

(if_statement
  condition: (comparison_operator (identifier) @_name (string) @_main)
  (#eq? @_name "__name__")
  (#match? @_main "__main__")) @entry

; Identifiers used as values in callback positions: arguments, literals, returns, assignments.
(argument_list (identifier) @name @reference.value)
(keyword_argument value: (identifier) @name @reference.value)
(argument_list (attribute object: (_) @qualifier attribute: (identifier) @name) @reference.value)
(return_statement (identifier) @name @reference.value)
(assignment right: (identifier) @name @reference.value)
(pair value: (identifier) @name @reference.value)
