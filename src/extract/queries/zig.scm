; Zig facts for the extraction engine (src/extract/engine.rs capture convention).

(function_declaration name: (identifier) @name) @definition.function
(variable_declaration (identifier) @name (struct_declaration)) @definition.struct
(variable_declaration (identifier) @name (enum_declaration)) @definition.enum
(test_declaration [(string) (identifier)] @name) @definition.function

(variable_declaration
  (identifier) @import.alias
  (builtin_function (builtin_identifier) @_import (arguments (string (string_content) @import.path)))
  (#eq? @_import "@import")) @import

; build.zig: `b.addModule("name", .{ .root_source_file = b.path("src/x.zig") })` (or the older
; `.{ .path = "src/x.zig" }`) defines a named module, recorded as a re-exported import.
(call_expression
  function: (field_expression member: (identifier) @_add @import.reexport)
  (string (string_content) @import.alias)
  (anonymous_struct_initializer
    (initializer_list
      (assignment_expression
        left: (field_expression member: (identifier) @_field)
        right: [
          (call_expression (string (string_content) @import.path))
          (anonymous_struct_initializer
            (initializer_list (assignment_expression right: (string (string_content) @import.path))))
        ])))
  (#eq? @_add "addModule")
  (#eq? @_field "root_source_file")) @import

; Declarations re-binding a member of an import: `const Request = @import("request.zig").Request;`
; and `const Request = request.Request;`.
(variable_declaration
  (identifier) @import.alias
  (field_expression
    object: (builtin_function
      (builtin_identifier) @_import
      (arguments (string (string_content) @import.path)))
    member: (identifier) @import.name)
  (#eq? @_import "@import")) @import
(variable_declaration
  (identifier) @import.alias
  (field_expression object: (identifier) @import.path member: (identifier) @import.name)) @import

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression object: (_) @qualifier member: (identifier) @name)) @reference.call

; Local bindings: `var s = mod.T{...}`, `var s: mod.T = ...`, `const s = T.init()`.
(variable_declaration (identifier) @binding.name (struct_initializer . (_) @binding.ctor)) @binding
(variable_declaration (identifier) @binding.name type: (_) @binding.type) @binding
(variable_declaration (identifier) @binding.name (call_expression function: (_) @binding.call)) @binding

; Types of parameters and results (`T`, `*T`, `mod.T`, `*mod.T`).
(parameter type: [
  (identifier) @name
  (pointer_type (identifier) @name)
  (field_expression object: (identifier) @qualifier member: (identifier) @name)
  ; `*mod.T` parses as a field of the pointer.
  (field_expression object: (pointer_type (identifier) @qualifier) member: (identifier) @name)
] @reference.type)
(function_declaration type: [
  (identifier) @name
  (pointer_type (identifier) @name)
  (field_expression object: (identifier) @qualifier member: (identifier) @name)
] @reference.type)

; Typed parameters and struct fields; `Self` and `@This()` stay with the enclosing struct.
(parameter name: (identifier) @binding.name type: (_) @binding.type
  (#not-match? @binding.type "Self|@This")) @binding
(container_field name: (identifier) @field.name type: (_) @field.type) @field
