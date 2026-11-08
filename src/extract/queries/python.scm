; Python facts for the extraction engine (src/extract/engine.rs capture convention).

[(assignment) (augmented_assignment) (for_statement) (named_expression)
 (as_pattern_target) (delete_statement) (parameters) (case_pattern)
 (lambda_parameters) (for_in_clause) (global_statement) (nonlocal_statement)
 (try_statement) (function_definition) (class_definition) (import_statement)
 (import_from_statement) (decorated_definition)] @python_post_parse

(function_definition name: (identifier) @name) @definition.function
(function_definition return_type: (type) @return.type) @return
(class_definition name: (identifier) @name) @definition.class
(class_definition superclasses: (argument_list (identifier) @name) @reference.inherit)
(class_definition
  superclasses: (argument_list (attribute object: (_) @qualifier attribute: (identifier) @name)) @reference.inherit)
((expression_statement (assignment left: (identifier) @name)) @_top @definition.variable (#parent? @_top "module"))
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
; A context target receives __enter__/__aenter__'s result, not the manager's type.
(with_item value: (as_pattern
  (call function: [(identifier) (attribute)] @binding.call)
  alias: (as_pattern_target (identifier) @binding.name))) @binding
(with_item value: (as_pattern
  [(identifier) (attribute)] @binding.call
  alias: (as_pattern_target (identifier) @binding.name))) @binding
(assignment
  left: (identifier) @binding.name
  right: (call function: [(identifier) (attribute)] @binding.ctor)
  (#match? @binding.ctor "(^|[.])[A-Z][A-Za-z0-9_]*$")) @binding
(assignment
  left: (identifier) @binding.name
  right: (call function: [(identifier) (attribute)] @binding.call)
  (#match? @binding.call "(^|[.])[a-z_][A-Za-z0-9_]*$")) @binding
(assignment
  left: (identifier) @binding.name
  right: (await (call function: [(identifier) (attribute)] @binding.call))) @binding

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
(default_parameter value: (identifier) @name @reference.value)
(typed_default_parameter value: (identifier) @name @reference.value)
(argument_list (attribute object: (_) @qualifier attribute: (identifier) @name) @reference.value)
(return_statement (identifier) @name @reference.value)
(assignment right: (identifier) @name @reference.value)
(pair value: (identifier) @name @reference.value)
(list (identifier) @name @reference.value @python_list_value)
(parameters (identifier) @python_list_local)
(parameters (list_splat_pattern) @python_list_local)
(parameters (dictionary_splat_pattern) @python_list_local)
(lambda_parameters (identifier) @python_list_lambda)
(lambda_parameters (list_splat_pattern) @python_list_lambda)
(lambda_parameters (dictionary_splat_pattern) @python_list_lambda)
(lambda_parameters (default_parameter name: (identifier) @python_list_lambda))
(default_parameter name: (_) @python_list_local)
(typed_parameter (identifier) @python_list_local)
(typed_default_parameter name: (_) @python_list_local)
(assignment left: (_) @python_list_local)
(augmented_assignment left: (_) @python_list_local)
(for_statement left: (_) @python_list_local)
(for_in_clause left: (_) @python_list_local)
(named_expression name: (_) @python_list_local)
(as_pattern_target (identifier) @python_list_local)
(import_statement) @python_list_local
(import_from_statement) @python_list_local
(delete_statement) @python_list_local
(case_pattern) @python_list_local
(global_statement (identifier) @python_list_global)
