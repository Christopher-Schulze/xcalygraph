; Verilog and SystemVerilog facts for the extraction engine (src/extract/engine.rs capture
; convention). Module instantiations are calls of the instantiated module.

(module_declaration (module_header (simple_identifier) @name)) @definition.class
(interface_declaration (interface_ansi_header (interface_identifier (simple_identifier) @name))) @definition.interface
(package_declaration (package_identifier (simple_identifier) @name)) @definition.module
(class_declaration . (class_identifier (simple_identifier) @name)) @definition.class
(class_declaration (class_type (class_identifier (simple_identifier) @name))) @reference.inherit
(function_declaration (function_body_declaration (function_identifier (function_identifier (simple_identifier) @name)))) @definition.function
(task_declaration (task_body_declaration (task_identifier (task_identifier (simple_identifier) @name)))) @definition.function

(include_compiler_directive (double_quoted_string) @import.path) @import
(package_import_item (package_identifier (simple_identifier) @import.path)) @import

(module_instantiation . (simple_identifier) @name) @reference.call
(tf_call . (simple_identifier) @name) @reference.call
