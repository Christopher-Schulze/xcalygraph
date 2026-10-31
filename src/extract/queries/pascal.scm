; Pascal and Delphi facts for the extraction engine (src/extract/engine.rs capture convention).
; Method declarations in class bodies are declarations of the `TType.Method` implementations.

(program (moduleName) @name) @definition.module
(program) @entry
(declType name: (identifier) @name type: (declClass)) @definition.class
(declType name: (identifier) @name type: (declIntf)) @definition.interface
(declType name: (identifier) @name type: (type (declEnum))) @definition.enum
(declClass (declSection (declProc name: (identifier) @name) @definition.method))
(declIntf (declSection (declProc name: (identifier) @name) @definition.method))
(declClass (declProc name: (identifier) @name) @definition.method)
(defProc header: (declProc name: (identifier) @name)) @definition.function
(defProc header: (declProc name: (genericDot lhs: (identifier) @owner rhs: (identifier) @name))) @definition.method
(declConsts (declConst name: (identifier) @name)) @definition.constant
(declClass parent: (typeref (identifier) @name)) @reference.inherit
(declClass (declSection (declField name: (identifier) @field.name type: (type) @field.type))) @field

; One import per unit name (`uses A, B`).
(declUses (moduleName) @import.path @import)

(exprCall entity: (identifier) @name) @reference.call
(exprCall entity: (exprDot lhs: (_) @qualifier rhs: (identifier) @name)) @reference.call
; Parameterless calls written as statements: `Helper;`, `FRows.Clear;`.
(statement (identifier) @name) @reference.call
(statement (exprDot lhs: (_) @qualifier rhs: (identifier) @name)) @reference.call
(typeref (identifier) @name) @reference.type
(declVar name: (identifier) @binding.name type: (type) @binding.type) @binding
(declArg name: (identifier) @binding.name type: (type) @binding.type) @binding

(exprCall
  entity: (identifier) @_getenv
  args: (exprArgs . (literalString) @env.name)
  (#match? @_getenv "^(?i)GetEnvironmentVariable$")) @env
