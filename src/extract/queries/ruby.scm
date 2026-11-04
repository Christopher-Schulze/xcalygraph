; Ruby facts for the extraction engine (src/extract/engine.rs capture convention). Modules map to
; traits (mixins and namespaces).

(class name: [(constant) @name (scope_resolution name: (constant) @name)]) @definition.class
(module name: [(constant) @name (scope_resolution name: (constant) @name)]) @definition.trait
(method name: (_) @name) @definition.method
(singleton_method name: (_) @name) @definition.method
((assignment left: (constant) @name) @definition.constant (#parent? @definition.constant "program"))
((assignment left: (constant) @name) @definition.constant (#parent? @definition.constant "body_statement"))

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
(then (identifier) @name @reference.value)
(else (identifier) @name @reference.value)
(block_body (identifier) @name @reference.value)
; Bare names in expressions are method calls unless a local of the method has that name.
(assignment right: (identifier) @name @reference.value)
(binary [left: (identifier) right: (identifier)] @name @reference.value)
(element_reference (identifier) @name @reference.value)
(if condition: (identifier) @name @reference.value)
(unless condition: (identifier) @name @reference.value)
(if_modifier condition: (identifier) @name @reference.value)
(unless_modifier condition: (identifier) @name @reference.value)
(assignment left: (identifier) @local)
(operator_assignment left: (identifier) @local)
(method_parameters (identifier) @local)
(lambda_parameters (identifier) @local)
(block_parameters (identifier) @local)
(optional_parameter name: (identifier) @local)
(keyword_parameter name: (identifier) @local)
(splat_parameter name: (identifier) @local)
(hash_splat_parameter name: (identifier) @local)
(block_parameter name: (identifier) @local)
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
