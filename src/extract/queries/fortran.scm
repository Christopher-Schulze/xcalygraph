; Fortran facts for the extraction engine (src/extract/engine.rs capture convention).

(module (module_statement (name) @name)) @definition.module
(submodule (submodule_statement (name) @name)) @definition.module
(program (program_statement (name) @name)) @definition.module
(program) @entry
(subroutine (subroutine_statement name: (name) @name)) @definition.function
(function (function_statement name: (name) @name)) @definition.function
(module_procedure (module_procedure_statement name: (name) @name)) @definition.function
(derived_type_definition (derived_type_statement (type_name) @name)) @definition.struct
(derived_type_statement (base_type_specifier (identifier) @name)) @reference.inherit

(use_statement (module_name) @import.path) @import
(include_statement path: (filename) @import.path) @import

(subroutine_call subroutine: (identifier) @name) @reference.call
(subroutine_call subroutine: (derived_type_member_expression (identifier) @qualifier (type_member) @name)) @reference.call
(call_expression . (identifier) @name) @reference.call
(derived_type name: (type_name) @name) @reference.type

(subroutine_call
  subroutine: (identifier) @_getenv
  (argument_list . (string_literal) @env.name)
  (#match? @_getenv "^(?i)get_environment_variable$")) @env
