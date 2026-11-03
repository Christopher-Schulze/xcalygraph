; JavaScript facts for the extraction engine (src/extract/engine.rs capture convention).

(program (lexical_declaration (variable_declarator name: (identifier) @name) @definition.constant))
(program (export_statement (lexical_declaration (variable_declarator name: (identifier) @name) @definition.constant)))
(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function
(program (lexical_declaration (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @definition.function))
(program (export_statement (lexical_declaration (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @definition.function)))
(program (variable_declaration (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @definition.function))
(class_declaration name: (identifier) @name) @definition.class
; CommonJS and object-member functions: `exports.f = function`, `module.exports.f = ...`,
; `app.handle = function handle()`; `Type.prototype.m = function` is a method of `Type`.
(program (expression_statement (assignment_expression
  left: (member_expression object: (identifier) @_exports property: (property_identifier) @name)
  right: [(function_expression) (arrow_function)]) @definition.function)
  (#eq? @_exports "exports"))
; `app.render = function`: a method of the object, not a bare name of the module.
(program (expression_statement (assignment_expression
  left: (member_expression object: (identifier) @owner property: (property_identifier) @name)
  right: [(function_expression) (arrow_function)]) @definition.method)
  (#not-eq? @owner "exports"))
(program (expression_statement (assignment_expression
  left: (member_expression
    object: (member_expression object: (identifier) @_module property: (property_identifier) @_exports)
    property: (property_identifier) @name)
  right: [(function_expression) (arrow_function)]) @definition.function)
  (#eq? @_module "module")
  (#eq? @_exports "exports"))
(program (expression_statement (assignment_expression
  left: (member_expression
    object: (member_expression object: (identifier) @owner property: (property_identifier) @_prototype)
    property: (property_identifier) @name)
  right: [(function_expression) (arrow_function)]) @definition.method)
  (#eq? @_prototype "prototype"))
(method_definition name: (property_identifier) @name) @definition.method
; ECMAScript private methods `#clone()` and their calls `this.#clone()`.
(method_definition name: (private_property_identifier) @name) @definition.method

(class_declaration (class_heritage (identifier) @name @reference.inherit))
(class_declaration (class_heritage (member_expression object: (_) @qualifier property: (property_identifier) @name) @reference.inherit))

(import_statement) @import.tree @import
(export_statement source: (string)) @import.tree @import
(variable_declarator
  name: (identifier) @import.alias
  value: (call_expression function: (identifier) @_require arguments: (arguments . (string) @import.path))
  (#eq? @_require "require")) @import
; `var compileETag = require('./utils').compileETag`: one member of a required module.
(variable_declarator
  name: (identifier) @import.alias
  value: (member_expression
    object: (call_expression function: (identifier) @_require arguments: (arguments . (string) @import.path))
    property: (property_identifier) @import.name)
  (#eq? @_require "require")) @import

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (member_expression object: (_) @qualifier property: (property_identifier) @name)) @reference.call
(call_expression function: (member_expression object: (_) @qualifier property: (private_property_identifier) @name)) @reference.call
(new_expression constructor: (identifier) @name) @reference.call
(new_expression constructor: (member_expression object: (_) @qualifier property: (property_identifier) @name)) @reference.call

(variable_declarator name: (identifier) @binding.name value: (new_expression constructor: (_) @binding.ctor)) @binding
; `require(...)` binds an import, not a local of unknown type.
(variable_declarator name: (identifier) @binding.name value: (call_expression function: (_) @binding.call) (#not-eq? @binding.call "require")) @binding
(variable_declarator name: (identifier) @binding.name value: (await_expression (call_expression function: (_) @binding.call))) @binding

(assignment_expression
  left: (member_expression object: (this) property: (property_identifier) @field.name)
  right: (new_expression constructor: (_) @field.type)) @field
(field_definition property: (property_identifier) @field.name value: (new_expression constructor: (_) @field.type)) @field

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

; Identifiers used as values in callback positions: arguments, literals, returns, assignments.
(arguments (identifier) @name @reference.value)
(arguments (member_expression object: (_) @qualifier property: (property_identifier) @name) @reference.value)
(pair value: (identifier) @name @reference.value)
(shorthand_property_identifier) @name @reference.value
(return_statement (identifier) @name @reference.value)
(variable_declarator value: (identifier) @name @reference.value)
(assignment_expression right: (identifier) @name @reference.value)
(export_statement (export_clause (export_specifier name: (identifier) @name @reference.value)))
(export_statement value: (identifier) @name @reference.value)
; A parenthesized expression statement only comes from Svelte markup (`onclick={save}`,
; `{$user.name}`, src/extract/sfc.rs): the identifier or member object is used as a value.
(expression_statement (parenthesized_expression (identifier) @name @reference.value))
(expression_statement
  (parenthesized_expression (member_expression object: (identifier) @name) @reference.value))

; Parameters and function locals shadow module names (`onResponse?.(x)` calls the parameter).
(formal_parameters (identifier) @local)
(formal_parameters (assignment_pattern left: (identifier) @local))
(arrow_function parameter: (identifier) @local)
(statement_block (lexical_declaration (variable_declarator name: (identifier) @local)))
(statement_block (variable_declaration (variable_declarator name: (identifier) @local)))
; Destructured parameters and locals (`({text, onResponse}) => ...`, `const [a, b] = ...`).
(object_pattern (shorthand_property_identifier_pattern) @local)
(object_pattern (pair_pattern value: (identifier) @local))
(object_pattern (object_assignment_pattern left: (shorthand_property_identifier_pattern) @local))
(array_pattern (identifier) @local)
