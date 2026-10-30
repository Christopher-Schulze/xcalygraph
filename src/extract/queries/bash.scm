; Bash facts for the extraction engine (src/extract/engine.rs capture convention).

(function_definition name: (word) @name) @definition.function

(command
  name: (command_name (word) @_source)
  argument: [(word) (string) (raw_string)] @import.tree
  (#any-of? @_source "source" ".")) @import

(command name: (command_name (word) @name)) @reference.call

(simple_expansion (variable_name) @env.name) @env
(expansion (variable_name) @env.name) @env
(variable_assignment name: (variable_name) @assigned)
(declaration_command (variable_name) @assigned)
(for_statement variable: (variable_name) @assigned)
