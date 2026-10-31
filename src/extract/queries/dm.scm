; DM (BYOND) facts for the extraction engine (src/extract/engine.rs capture convention). Type
; paths nest (`/obj/item/store` is `obj` > `item` > `store`), procs belong to their type.

(type_definition root: (identifier) @name) @definition.class
(subtype_definition (identifier) @name) @definition.class
(proc_definition name: (identifier) @name) @definition.function
(proc_override name: (identifier) @name) @definition.function

(preproc_include path: (_) @import.path) @import

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression argument: (builtin_vars) field: (identifier) @name)) @reference.call
(call_expression function: (field_expression argument: (identifier) @qualifier field: (identifier) @name)) @reference.call
