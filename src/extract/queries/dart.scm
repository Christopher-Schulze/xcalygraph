; Dart facts for the extraction engine (src/extract/engine.rs capture convention).

(class_declaration name: (identifier) @name) @definition.class
(mixin_declaration (identifier) @name) @definition.trait
(extension_declaration name: (identifier) @name) @definition.class
(enum_declaration name: (identifier) @name) @definition.enum
(enum_entry name: (identifier) @name) @definition.constant
(type_alias (type_identifier) @name) @definition.type
(function_declaration signature: (function_signature name: (identifier) @name)) @definition.function
(function_declaration signature: (function_signature return_type: (type) @return.type)) @return
(method_declaration signature: (method_signature (function_signature name: (identifier) @name))) @definition.method
(method_declaration signature: (method_signature (function_signature return_type: (type) @return.type))) @return
(method_declaration signature: (method_signature (getter_signature name: (identifier) @name))) @definition.method
; Abstract members and constructors without body.
(declaration (function_signature name: (identifier) @name)) @definition.method
(declaration (constructor_signature name: (identifier) @name)) @definition.method
(method_declaration signature: (method_signature (setter_signature name: (identifier) @name))) @definition.method
(method_declaration signature: (method_signature (constructor_signature name: (identifier) @name))) @definition.method
(method_declaration signature: (method_signature (factory_constructor_signature name: (identifier) @name))) @definition.method
(top_level_variable_declaration
  (initialized_identifier_list (initialized_identifier name: (identifier) @name))) @definition.variable
(top_level_variable_declaration
  (static_final_declaration_list (static_final_declaration name: (identifier) @name))) @definition.constant

(library_import (import_specification uri: (configurable_uri (uri (string_literal) @import.path)))) @import
(library_import
  (import_specification uri: (configurable_uri (uri (string_literal) @import.path)) alias: (identifier) @import.alias)) @import
; `export` and `part` re-export to importers of this library; `part of` names the library.
(library_export "export" @import.reexport uri: (configurable_uri (uri (string_literal) @import.path))) @import
(part_directive "part" @import.reexport uri: (uri (string_literal) @import.path)) @import
(part_of_directive (uri (string_literal) @import.path)) @import

(superclass type: (type . (type_identifier) @name)) @reference.inherit
(interfaces (type . (type_identifier) @name)) @reference.implement
(mixins (type . (type_identifier) @name)) @reference.implement

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (member_expression object: (_) @qualifier property: (identifier) @name)) @reference.call
(call_expression function: (null_aware_member_expression object: (_) @qualifier property: (identifier) @name)) @reference.call
(new_expression type: (type . (type_identifier) @name)) @reference.call
(const_object_expression type: (type . (type_identifier) @name)) @reference.call
(type (type_identifier) @name) @reference.type

(formal_parameter (type) @binding.type name: (identifier) @binding.name) @binding
(initialized_variable_declaration
  name: (identifier) @binding.name
  value: (call_expression function: (identifier) @binding.ctor)
  (#match? @binding.ctor "^[A-Z]")) @binding
(initialized_variable_declaration
  name: (identifier) @binding.name
  value: (call_expression function: (identifier) @binding.call)
  (#match? @binding.call "^[a-z_]")) @binding
(initialized_variable_declaration (type) @binding.type name: (identifier) @binding.name) @binding
(class_body
  (member_declaration
    (declaration (type) @field.type (initialized_identifier_list (initialized_identifier name: (identifier) @field.name))))) @field

; Identifiers used as values: arguments.
(arguments (identifier) @name @reference.value)
