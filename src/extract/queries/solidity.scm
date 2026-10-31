; Solidity facts for the extraction engine (src/extract/engine.rs capture convention).

(contract_declaration name: (identifier) @name) @definition.class
(interface_declaration name: (identifier) @name) @definition.interface
(library_declaration name: (identifier) @name) @definition.class
(struct_declaration name: (identifier) @name) @definition.struct
(enum_declaration name: (identifier) @name) @definition.enum
(event_definition name: (identifier) @name) @definition.type
(error_declaration name: (identifier) @name) @definition.type
(function_definition name: (identifier) @name) @definition.function
(modifier_definition name: (identifier) @name) @definition.function
(state_variable_declaration name: (identifier) @name) @definition.variable
(state_variable_declaration type: (type_name) @field.type name: (identifier) @field.name) @field
(inheritance_specifier ancestor: (user_defined_type (identifier) @name)) @reference.inherit

(import_directive source: (string) @import.path) @import

(call_expression function: (expression (identifier) @name)) @reference.call
(call_expression function: (expression (member_expression object: (_) @qualifier property: (identifier) @name))) @reference.call
(emit_statement name: (expression (identifier) @name)) @reference.call
(revert_statement error: (expression (identifier) @name)) @reference.call
(modifier_invocation (identifier) @name) @reference.call
(user_defined_type (identifier) @name) @reference.type

(parameter type: (type_name (user_defined_type (identifier) @binding.type)) name: (identifier) @binding.name) @binding
(variable_declaration type: (type_name (user_defined_type (identifier) @binding.type)) name: (identifier) @binding.name) @binding
