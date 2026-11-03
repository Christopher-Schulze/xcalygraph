; Rust facts for the extraction engine (src/extract/engine.rs capture convention).

(function_item name: (identifier) @name) @definition.function
(function_item return_type: (_) @return.type) @return
(function_signature_item name: (identifier) @name) @definition.function
(function_signature_item return_type: (_) @return.type) @return
(struct_item name: (type_identifier) @name) @definition.struct
(union_item name: (type_identifier) @name) @definition.struct
(enum_item name: (type_identifier) @name) @definition.enum
(trait_item name: (type_identifier) @name) @definition.trait
(type_item name: (type_identifier) @name) @definition.type
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.variable
(macro_definition name: (identifier) @name) @definition.macro
(mod_item name: (identifier) @name) @definition.module

((attribute_item
  (attribute
    (identifier) @_attribute
    value: [(string_literal) (raw_string_literal)] @name))
 (#eq? @_attribute "path")) @reference.path

(impl_item type: (_) @impl.type) @impl
(impl_item trait: (_) @impl.trait type: (_) @impl.type) @impl
(trait_item bounds: (trait_bounds [(type_identifier) (scoped_type_identifier)] @name) @reference.inherit)

; Generic parameter names are scoped markers; bounded references connect each parameter to its
; source trait without treating it as a concrete implementation.
(function_item
  type_parameters: (type_parameters
    (type_parameter name: (type_identifier) @name) @reference.implement))
(function_item
  type_parameters: (type_parameters
    (type_parameter
      name: (type_identifier) @qualifier
      bounds: (trait_bounds [(type_identifier) (scoped_type_identifier) (generic_type)] @name)
        @reference.implement)))
(function_item
  (where_clause
    (where_predicate
      left: (type_identifier) @qualifier
      bounds: (trait_bounds [(type_identifier) (scoped_type_identifier) (generic_type)] @name)
        @reference.implement)))
(impl_item
  type_parameters: (type_parameters
    (type_parameter name: (type_identifier) @name) @reference.implement)
  body: (declaration_list (function_item name: (identifier) @generic.scope)))
(impl_item
  type_parameters: (type_parameters
    (type_parameter
      name: (type_identifier) @qualifier
      bounds: (trait_bounds [(type_identifier) (scoped_type_identifier) (generic_type)] @name)
        @reference.implement))
  body: (declaration_list (function_item name: (identifier) @generic.scope)))
(impl_item
  (where_clause
    (where_predicate
      left: (type_identifier) @qualifier
      bounds: (trait_bounds [(type_identifier) (scoped_type_identifier) (generic_type)] @name)
        @reference.implement))
  body: (declaration_list (function_item name: (identifier) @generic.scope)))

(use_declaration (visibility_modifier) @import.reexport argument: (_) @import.tree) @import
(use_declaration argument: (_) @import.tree) @import
(extern_crate_declaration name: (identifier) @import.path) @import

(call_expression function: (identifier) @name) @reference.call
(call_expression
  function: (scoped_identifier path: (_) @qualifier name: (identifier) @name)) @reference.call
(call_expression
  function: (field_expression value: (_) @qualifier field: (field_identifier) @name)) @reference.call
(call_expression
  function: (generic_function function: (identifier) @name)) @reference.call
(call_expression
  function: (generic_function
    function: (scoped_identifier path: (_) @qualifier name: (identifier) @name))) @reference.call
(call_expression
  function: (generic_function
    function: (field_expression value: (_) @qualifier field: (field_identifier) @name))) @reference.call
(macro_invocation macro: (identifier) @name) @reference.call
(macro_invocation
  macro: (scoped_identifier path: (_) @qualifier name: (identifier) @name)) @reference.call
(type_identifier) @name @reference.type
(scoped_type_identifier path: (_) @qualifier name: (type_identifier) @name) @reference.type

(parameter pattern: (identifier) @binding.name type: (_) @binding.type) @binding
(let_declaration pattern: (identifier) @binding.name) @binding
(let_declaration pattern: (identifier) @binding.name type: (_) @binding.type) @binding
(let_declaration
  pattern: (identifier) @binding.name
  value: (struct_expression name: (_) @binding.ctor)) @binding
(let_declaration
  pattern: (identifier) @binding.name
  value: [(call_expression function: (scoped_identifier path: (_) @binding.ctor name: (identifier) @_ctor))
          (try_expression (call_expression function: (scoped_identifier path: (_) @binding.ctor name: (identifier) @_ctor)))]
  (#match? @binding.ctor "(^|::)[A-Z][A-Za-z0-9_]*$")
  (#match? @_ctor "^(new|default|from|with_.*|open|create|build|load|parse|connect)$")) @binding
(let_declaration
  pattern: (identifier) @binding.name
  value: [(call_expression function: (identifier) @binding.call)
          (try_expression (call_expression function: (identifier) @binding.call))]) @binding

(field_declaration name: (field_identifier) @field.name type: (_) @field.type) @field

(call_expression
  function: (scoped_identifier path: (_) @_path name: (identifier) @_fn)
  arguments: (arguments . (string_literal) @env.name)
  (#match? @_path "(^|::)env$")
  (#any-of? @_fn "var" "var_os")) @env
(macro_invocation
  macro: (identifier) @_macro
  (token_tree . (string_literal) @env.name)
  (#any-of? @_macro "env" "option_env")) @env

; Identifiers used as values in callback positions: arguments, literals, returns, assignments.
(arguments (identifier) @name @reference.value)
(arguments (scoped_identifier path: (_) @qualifier name: (identifier) @name) @reference.value)
(field_initializer value: (identifier) @name @reference.value)
(let_declaration value: (identifier) @name @reference.value)
(return_expression (identifier) @name @reference.value)

; Calls and values inside macro arguments (`vec![f(x)]`, `assert_eq!(parse(x), Store::open(p))`):
; the token tree is unparsed, so an identifier directly followed by a parenthesized group is a
; call (the group is the call node, so its size is the argument count), `recv.method(...)` and
; `Path::name(...)` qualified calls.
((token_tree (identifier) @name . (token_tree) @reference.call)
 (#match? @reference.call "^\\("))
((token_tree (identifier) @qualifier . "." . (identifier) @name . (token_tree) @reference.call)
 (#match? @reference.call "^\\("))
((token_tree (identifier) @qualifier . "::" . [(identifier) (primitive_type)] @name . (token_tree) @reference.call)
 (#match? @reference.call "^\\("))
(shorthand_field_initializer (identifier) @name @reference.value)
; `Enum::Variant` in expressions and patterns names the enum.
(scoped_identifier path: (identifier) @name) @reference.type
(tuple_struct_pattern type: (scoped_identifier path: (identifier) @name)) @reference.type
