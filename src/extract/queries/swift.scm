; Swift facts for the extraction engine (src/extract/engine.rs capture convention). Members of an
; `extension T` name `T` as owner; the extension itself is no symbol.

(class_declaration declaration_kind: "class" name: (type_identifier) @name) @definition.class
(class_declaration declaration_kind: "actor" name: (type_identifier) @name) @definition.class
(class_declaration declaration_kind: "struct" name: (type_identifier) @name) @definition.struct
(class_declaration declaration_kind: "enum" name: (type_identifier) @name) @definition.enum
(protocol_declaration name: (type_identifier) @name) @definition.interface
(enum_entry name: (simple_identifier) @name) @definition.constant
(typealias_declaration name: (type_identifier) @name) @definition.type
; The result type is the direct `user_type` child (parameters are `parameter` nodes).
(function_declaration name: (simple_identifier) @name (user_type (type_identifier) @return.type)?) @definition.function
(init_declaration "init" @name) @definition.method
(protocol_function_declaration name: (simple_identifier) @name) @definition.method
; Members of `extension T` belong to `T` (implementation block, like a Rust `impl`).
(class_declaration declaration_kind: "extension" name: (user_type (type_identifier) @impl.type)) @impl
(source_file (property_declaration name: (pattern (simple_identifier) @name)) @definition.variable)
; Parameter types sit in repeated `name` fields: match them positionally.

(import_declaration (identifier) @import.path) @import

(inheritance_specifier inherits_from: (user_type (type_identifier) @name)) @reference.implement

(call_expression (simple_identifier) @name (call_suffix)) @reference.call
(call_expression
  (navigation_expression target: (_) @qualifier suffix: (navigation_suffix suffix: (simple_identifier) @name))
  (call_suffix)) @reference.call
(user_type (type_identifier) @name) @reference.type
(attribute (user_type (type_identifier) @name)) @reference.type

(parameter (simple_identifier) @binding.name . (user_type) @binding.type) @binding
(property_declaration
  name: (pattern (simple_identifier) @binding.name)
  (type_annotation type: (user_type) @binding.type)) @binding
(property_declaration
  name: (pattern (simple_identifier) @binding.name)
  value: (call_expression (simple_identifier) @binding.ctor)
  (#match? @binding.ctor "^[A-Z]")) @binding
(property_declaration
  name: (pattern (simple_identifier) @binding.name)
  value: (call_expression (simple_identifier) @binding.call)
  (#match? @binding.call "^[a-z_]")) @binding

(class_body
  (property_declaration
    name: (pattern (simple_identifier) @field.name)
    (type_annotation type: (user_type) @field.type)) @field)

; Identifiers used as values: arguments.
(value_argument value: (simple_identifier) @name @reference.value)
(value_argument value: (navigation_expression target: (simple_identifier) @qualifier suffix: (navigation_suffix suffix: (simple_identifier) @name)) @reference.value)
