; C facts for the extraction engine (src/extract/engine.rs capture convention). Prototypes are
; definitions without a body and get the declaration flag (src/extract/lang/c.rs).

(function_definition declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition declarator: (function_declarator declarator: (parenthesized_declarator (identifier) @name))) @definition.function
(function_definition
  declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @definition.function
; Function declarations anywhere (prototypes inside bodies are rare); file-level variables at
; file level, in include guards and in namespaces or `extern "C"`.
(declaration declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(declaration declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(function_definition type: (type_identifier) @return.type) @return
(function_definition type: (struct_specifier name: (type_identifier) @return.type)) @return
(struct_specifier name: (type_identifier) @name body: (_)) @definition.struct
(union_specifier name: (type_identifier) @name body: (_)) @definition.struct
(enum_specifier name: (type_identifier) @name body: (_)) @definition.enum
(enumerator name: (identifier) @name) @definition.constant
(type_definition declarator: (type_identifier) @name) @definition.type
(type_definition declarator: (pointer_declarator declarator: (type_identifier) @name)) @definition.type
(preproc_def name: (identifier) @name) @definition.macro
(preproc_function_def name: (identifier) @name) @definition.macro
((declaration declarator: (init_declarator declarator: (identifier) @name)) @_top @definition.variable (#parent? @_top "translation_unit"))
(preproc_ifdef (declaration declarator: (init_declarator declarator: (identifier) @name)) @definition.variable)
(declaration_list (declaration declarator: (init_declarator declarator: (identifier) @name)) @definition.variable)
((declaration declarator: (identifier) @name) @_top @definition.variable (#parent? @_top "translation_unit"))
(preproc_ifdef (declaration declarator: (identifier) @name) @definition.variable)
(declaration_list (declaration declarator: (identifier) @name) @definition.variable)

; Lua-style `API int (name) (...)` declarations.
(declaration declarator: (function_declarator declarator: (parenthesized_declarator (identifier) @name))) @definition.function
(declaration declarator: (pointer_declarator declarator: (function_declarator declarator: (parenthesized_declarator (identifier) @name)))) @definition.function

; File-level tables (`static const Reg funcs[] = {...}`): their entries are references.
((declaration declarator: (init_declarator declarator: (array_declarator declarator: (identifier) @name))) @_top @definition.variable (#parent? @_top "translation_unit"))
(preproc_ifdef (declaration declarator: (init_declarator declarator: (array_declarator declarator: (identifier) @name))) @definition.variable)
(declaration_list (declaration declarator: (init_declarator declarator: (array_declarator declarator: (identifier) @name))) @definition.variable)

(preproc_include path: (_) @import.tree) @import

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression argument: (_) @qualifier field: (field_identifier) @name)) @reference.call
(type_identifier) @name @reference.type

(parameter_declaration type: (_) @binding.type declarator: (identifier) @binding.name) @binding
(parameter_declaration type: (_) @binding.type declarator: (pointer_declarator declarator: (identifier) @binding.name)) @binding
(declaration type: (_) @binding.type declarator: (identifier) @binding.name) @binding
(declaration type: (_) @binding.type declarator: (pointer_declarator declarator: (identifier) @binding.name)) @binding
(declaration type: (_) @binding.type declarator: (init_declarator declarator: (identifier) @binding.name)) @binding
(declaration
  type: (_) @binding.type
  declarator: (init_declarator declarator: (pointer_declarator declarator: (identifier) @binding.name))) @binding

(field_declaration type: (_) @field.type declarator: (field_identifier) @field.name) @field
(field_declaration type: (_) @field.type declarator: (pointer_declarator declarator: (field_identifier) @field.name)) @field

(call_expression
  function: (identifier) @_getenv
  arguments: (argument_list . (string_literal) @env.name)
  (#any-of? @_getenv "getenv" "secure_getenv")) @env

; Identifiers used as values: arguments (function pointers, constants), returns, initializers.
(argument_list (identifier) @name @reference.value)
(return_statement (identifier) @name @reference.value)
(init_declarator value: (identifier) @name @reference.value)
(initializer_list (identifier) @name @reference.value)
(binary_expression [left: (identifier) right: (identifier)] @name @reference.value)
(case_statement value: (identifier) @name @reference.value)
(argument_list (pointer_expression argument: (identifier) @name) @reference.value)
(initializer_list (pointer_expression argument: (identifier) @name) @reference.value)
(assignment_expression right: (identifier) @name @reference.value)
(assignment_expression right: (pointer_expression argument: (identifier) @name) @reference.value)
(conditional_expression consequence: (identifier) @name @reference.value)
(conditional_expression alternative: (identifier) @name @reference.value)
