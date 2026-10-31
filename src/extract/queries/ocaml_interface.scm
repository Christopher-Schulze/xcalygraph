; OCaml interface (`.mli`) facts: specifications are declarations of the implementation's
; definitions.

(value_specification (value_name) @name) @definition.function
(external (value_name) @name) @definition.function
(module_definition (module_binding (module_name) @name)) @definition.module
(module_type_definition (module_type_name) @name) @definition.interface
(type_definition (type_binding name: (type_constructor) @name)) @definition.type
(open_module module: (module_path) @import.path) @import
(type_constructor_path . (type_constructor) @name) @reference.type
(type_constructor_path (extended_module_path) @qualifier (type_constructor) @name) @reference.type
