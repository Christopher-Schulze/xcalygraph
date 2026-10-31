; Ruby facts for the extraction engine (src/extract/engine.rs capture convention). Modules map to
; traits (mixins and namespaces).

(class name: [(constant) @name (scope_resolution name: (constant) @name)]) @definition.class
(module name: [(constant) @name (scope_resolution name: (constant) @name)]) @definition.trait
(method name: (_) @name) @definition.method
(singleton_method name: (_) @name) @definition.method
(program (assignment left: (constant) @name) @definition.constant)
(body_statement (assignment left: (constant) @name) @definition.constant)

(class superclass: (superclass (constant) @name)) @reference.inherit
(class superclass: (superclass (scope_resolution scope: (_) @qualifier name: (constant) @name))) @reference.inherit
(call
  method: (identifier) @_mixin
  arguments: (argument_list (constant) @name)
  (#any-of? @_mixin "include" "extend" "prepend")) @reference.implement

(call
  method: (identifier) @_require
  arguments: (argument_list (string (string_content) @import.path))
  (#any-of? @_require "require" "require_relative" "load")) @import

(call !receiver method: (identifier) @name) @reference.call
(call receiver: (_) @qualifier method: (identifier) @name) @reference.call
; Without parentheses or receiver a call looks like a variable: statements and predicate or bang
; names are calls.
(body_statement (identifier) @name @reference.call)
((identifier) @name @reference.call (#match? @name "[?!]$"))
(scope_resolution scope: (_) @qualifier name: (constant) @name) @reference.type
(constant) @name @reference.type

(assignment
  left: (identifier) @binding.name
  right: (call receiver: [(constant) (scope_resolution)] @binding.ctor method: (identifier) @_new (#eq? @_new "new"))) @binding

(element_reference object: (constant) @_env (string (string_content) @env.name) (#eq? @_env "ENV")) @env
(call
  receiver: (constant) @_env
  method: (identifier) @_fetch
  arguments: (argument_list . (string (string_content) @env.name))
  (#eq? @_env "ENV")
  (#any-of? @_fetch "fetch" "key?")) @env

; Identifiers used as values: arguments.
(argument_list (identifier) @name @reference.value)
