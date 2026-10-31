; Go facts for the extraction engine (src/extract/engine.rs capture convention).

(package_clause (package_identifier) @package)

(function_declaration name: (identifier) @name) @definition.function
(function_declaration result: (_) @return.type) @return
(method_declaration
  receiver: (parameter_list (parameter_declaration type: (_) @owner))
  name: (field_identifier) @name) @definition.method
(method_declaration result: (_) @return.type) @return
(type_spec name: (type_identifier) @name type: (struct_type)) @definition.struct
(type_spec name: (type_identifier) @name type: (interface_type)) @definition.interface
(type_spec
  name: (type_identifier) @name
  type: [(type_identifier) (qualified_type) (pointer_type) (function_type) (map_type)
         (slice_type) (array_type) (channel_type) (generic_type)]) @definition.type
(type_alias name: (type_identifier) @name) @definition.type
(interface_type (method_elem name: (field_identifier) @name) @definition.method)
(source_file (const_declaration (const_spec name: (identifier) @name) @definition.constant))
(source_file (var_declaration (var_spec name: (identifier) @name) @definition.variable))
(source_file (var_declaration (var_spec_list (var_spec name: (identifier) @name) @definition.variable)))

(import_spec path: (_) @import.path) @import
(import_spec name: (_) @import.alias path: (_) @import.path) @import

(call_expression function: (identifier) @name) @reference.call
(call_expression
  function: (selector_expression operand: (_) @qualifier field: (field_identifier) @name)) @reference.call
(type_identifier) @name @reference.type
(qualified_type package: (package_identifier) @qualifier name: (type_identifier) @name) @reference.type
(composite_literal type: (type_identifier) @name) @reference.type
(interface_type (type_elem (type_identifier) @name) @reference.inherit)
(struct_type (field_declaration_list (field_declaration !name type: (type_identifier) @name) @reference.inherit))

(parameter_declaration name: (identifier) @binding.name type: (_) @binding.type) @binding
(var_spec name: (identifier) @binding.name type: (_) @binding.type) @binding
(short_var_declaration
  left: (expression_list . (identifier) @binding.name)
  right: (expression_list . (composite_literal type: (_) @binding.ctor))) @binding
(short_var_declaration
  left: (expression_list . (identifier) @binding.name)
  right: (expression_list . (unary_expression operand: (composite_literal type: (_) @binding.ctor)))) @binding
(short_var_declaration
  left: (expression_list . (identifier) @binding.name)
  right: (expression_list . (call_expression function: (_) @binding.call))) @binding
(var_spec
  name: (identifier) @binding.name
  value: (expression_list . (call_expression function: (_) @binding.call))) @binding

(field_declaration name: (field_identifier) @field.name type: (_) @field.type) @field

(call_expression
  function: (selector_expression operand: (identifier) @_pkg field: (field_identifier) @_fn)
  arguments: (argument_list . [(interpreted_string_literal) (raw_string_literal)] @env.name)
  (#eq? @_pkg "os")
  (#any-of? @_fn "Getenv" "LookupEnv")) @env

; Identifiers used as values in callback positions: arguments, literals, returns, assignments.
(argument_list (identifier) @name @reference.value)
(argument_list (selector_expression operand: (identifier) @qualifier field: (field_identifier) @name) @reference.value)
(return_statement (expression_list (identifier) @name @reference.value))
(assignment_statement right: (expression_list (identifier) @name @reference.value))
(short_var_declaration right: (expression_list (identifier) @name @reference.value))
(var_spec value: (expression_list (identifier) @name @reference.value))
(const_spec value: (expression_list (identifier) @name @reference.value))
(var_spec value: (expression_list (selector_expression operand: (identifier) @qualifier field: (field_identifier) @name) @reference.value))
; Method values and function values bound to a name: `read := rw.Read`, `h = srv.Handle`.
(short_var_declaration right: (expression_list (selector_expression operand: (identifier) @qualifier field: (field_identifier) @name) @reference.value))
(assignment_statement right: (expression_list (selector_expression operand: (identifier) @qualifier field: (field_identifier) @name) @reference.value))
(const_spec value: (expression_list (selector_expression operand: (identifier) @qualifier field: (field_identifier) @name) @reference.value))

; Struct literal keys are field names; map literal keys are values.
(keyed_element value: (literal_element (identifier) @name @reference.value))
(literal_value (literal_element (identifier) @name @reference.value))
(composite_literal
  type: (map_type)
  body: (literal_value (keyed_element key: (literal_element (identifier) @name @reference.value))))
(literal_element (selector_expression operand: (identifier) @qualifier field: (field_identifier) @name) @reference.value)

; `for _, v := range xs`: v has the element type of xs.
(for_statement
  (range_clause
    left: (expression_list (_) (identifier) @binding.name)
    right: (identifier) @binding.range)) @binding

; Locals and parameters shadow package names in their function.
(short_var_declaration left: (expression_list (identifier) @local))
(parameter_declaration name: (identifier) @local)
(range_clause left: (expression_list (identifier) @local))
(block (statement_list (var_declaration (var_spec name: (identifier) @local))))
