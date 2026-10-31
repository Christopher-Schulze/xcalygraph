; Pascal and Delphi facts for the extraction engine (src/extract/engine.rs capture convention).
; Method declarations in class bodies are declarations of the `TType.Method` implementations.

(program (moduleName) @name) @definition.module
(program) @entry
(declType name: (identifier) @name type: (declClass)) @definition.class
(declType name: (identifier) @name type: (declIntf)) @definition.interface
(declType name: (identifier) @name type: (type (declEnum))) @definition.enum
; Generic types: `TList<T> = class`, `IRoute<T> = interface`.
(declType name: (genericTpl entity: (identifier) @name) type: (declClass)) @definition.class
(declType name: (genericTpl entity: (identifier) @name) type: (declIntf)) @definition.interface
(declClass (declSection (declProc name: (identifier) @name) @definition.method))
(declIntf (declSection (declProc name: (identifier) @name) @definition.method))
(declClass (declProc name: (identifier) @name) @definition.method)
(defProc header: (declProc name: (identifier) @name)) @definition.function
(defProc header: (declProc name: (genericDot lhs: (identifier) @owner rhs: (identifier) @name))) @definition.method
(defProc header: (declProc name: (genericDot lhs: (genericTpl entity: (identifier) @owner) rhs: (identifier) @name))) @definition.method
(declConsts (declConst name: (identifier) @name)) @definition.constant
(declClass parent: (typeref (identifier) @name)) @reference.inherit
(declClass parent: (typeref (typerefTpl entity: (identifier) @name))) @reference.inherit
(declIntf parent: (typeref (identifier) @name)) @reference.inherit
(declIntf parent: (typeref (typerefTpl entity: (identifier) @name))) @reference.inherit
(declClass (declSection (declField name: (identifier) @field.name type: (type) @field.type))) @field

; One import per unit name (`uses A, B`).
(declUses (moduleName) @import.path @import)

(exprCall entity: (identifier) @name) @reference.call
(exprCall entity: (exprDot lhs: (_) @qualifier rhs: (identifier) @name)) @reference.call
; Parameterless calls written as statements: `Helper;`, `FRows.Clear;`.
(statement (identifier) @name) @reference.call
(statement (exprDot lhs: (_) @qualifier rhs: (identifier) @name)) @reference.call
(typeref (identifier) @name) @reference.type
; Bare names in expressions: parameterless calls (`X := GetPort`), constants and functions
; passed on; locals resolve to nothing.
(assignment rhs: (identifier) @name @reference.value)
(exprArgs (identifier) @name @reference.value)
(exprBinary [lhs: (identifier) rhs: (identifier)] @name @reference.value)
(if condition: (identifier) @name @reference.value)
(assignment rhs: (exprDot lhs: (_) @qualifier rhs: (identifier) @name)) @reference.call
(typerefTpl entity: (identifier) @name) @reference.type
; `TList<T>.Create`: the generic type qualifies the call.
(statement (exprDot lhs: (exprTpl entity: (identifier) @qualifier) rhs: (identifier) @name)) @reference.call
(exprCall entity: (exprDot lhs: (exprTpl entity: (identifier) @qualifier) rhs: (identifier) @name)) @reference.call
(declVar name: (identifier) @binding.name type: (type) @binding.type) @binding
(declArg name: (identifier) @binding.name type: (type) @binding.type) @binding

(exprCall
  entity: (identifier) @_getenv
  args: (exprArgs . (literalString) @env.name)
  (#match? @_getenv "^(?i)GetEnvironmentVariable$")) @env
