; Robot Framework facts for the extraction engine (src/extract/engine.rs capture convention).

(keyword_definition (name) @name) @definition.function
(test_case_definition (name) @name) @definition.function
(setting_statement
  name: (setting_name) @_setting
  (arguments . (argument) @import.path)
  (#match? @_setting "^(?i)(Resource|Library|Variables)$")) @import
(keyword_invocation (keyword) @name) @reference.call
