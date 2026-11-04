; OCaml implementation facts for the extraction engine (src/extract/engine.rs capture
; convention). Only structure-level `let` bindings are symbols (no locals).

((value_definition (let_binding pattern: (value_name) @name)) @_top @definition.constant (#parent? @_top "compilation_unit"))
(structure (value_definition (let_binding pattern: (value_name) @name)) @definition.constant)
((value_definition (let_binding pattern: (value_name) @name (parameter))) @_top @definition.function (#parent? @_top "compilation_unit"))
(structure (value_definition (let_binding pattern: (value_name) @name (parameter))) @definition.function)
((value_definition (let_binding pattern: (value_name) @name body: [(fun_expression) (function_expression)])) @_top @definition.function (#parent? @_top "compilation_unit"))
(structure (value_definition (let_binding pattern: (value_name) @name body: [(fun_expression) (function_expression)])) @definition.function)
(external (value_name) @name) @definition.function
(module_definition (module_binding (module_name) @name)) @definition.module
(module_type_definition (module_type_name) @name) @definition.interface
(type_definition (type_binding name: (type_constructor) @name)) @definition.type
(class_definition (class_binding (class_name) @name)) @definition.class
(method_definition (method_name) @name) @definition.method
(exception_definition (constructor_declaration (constructor_name) @name)) @definition.type

(open_module module: (module_path) @import.path) @import
(include_module module: (module_path) @import.path) @import

(application_expression function: (value_path . (value_name) @name)) @reference.call
(application_expression function: (value_path (module_path) @qualifier (value_name) @name)) @reference.call
(infix_expression operator: (rel_operator) @_pipe right: (value_path . (value_name) @name) (#eq? @_pipe "|>")) @reference.call
(infix_expression operator: (rel_operator) @_pipe right: (value_path (module_path) @qualifier (value_name) @name) (#eq? @_pipe "|>")) @reference.call
(method_invocation (method_name) @name) @reference.call
(application_expression argument: (value_path . (value_name) @name) @reference.value)
(application_expression argument: (value_path (module_path) @qualifier (value_name) @name) @reference.value)
(type_constructor_path . (type_constructor) @name) @reference.type
(type_constructor_path (extended_module_path) @qualifier (type_constructor) @name) @reference.type

(application_expression
  function: (value_path (module_path (module_name) @_sys) (value_name) @_getenv)
  argument: (string (string_content) @env.name)
  (#eq? @_sys "Sys")
  (#match? @_getenv "^getenv")) @env
