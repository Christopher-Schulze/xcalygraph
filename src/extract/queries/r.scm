; R facts for the extraction engine (src/extract/engine.rs capture convention).

(binary_operator lhs: (identifier) @name operator: ["<-" "=" "<<-"] rhs: (function_definition)) @definition.function
(binary_operator lhs: (string (string_content) @name) operator: ["<-" "="] rhs: (function_definition)) @definition.function

(call
  function: (identifier) @_source
  arguments: (arguments . (argument value: (string (string_content) @import.path)))
  (#any-of? @_source "source" "sys.source")) @import
(call
  function: (identifier) @_library
  arguments: (arguments . (argument value: [(identifier) (string (string_content))] @import.path))
  (#any-of? @_library "library" "require" "requireNamespace")) @import

(call function: (identifier) @name) @reference.call
(call function: (namespace_operator lhs: (identifier) @qualifier rhs: (identifier) @name)) @reference.call
; Functions passed by name: `lapply(xs, helper)`, `do.call(helper, args)`.
(arguments (argument value: (identifier) @name) @reference.value)

(call
  function: (identifier) @_getenv
  arguments: (arguments . (argument value: (string (string_content) @env.name)))
  (#eq? @_getenv "Sys.getenv")) @env
