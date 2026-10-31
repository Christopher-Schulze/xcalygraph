
; JSX component usage counts as a call of the component; lowercase tags are HTML elements.
(jsx_opening_element name: (identifier) @name (#match? @name "^[A-Z]")) @reference.call
(jsx_self_closing_element name: (identifier) @name (#match? @name "^[A-Z]")) @reference.call
(jsx_opening_element name: (member_expression object: (_) @qualifier property: (property_identifier) @name)) @reference.call
(jsx_self_closing_element name: (member_expression object: (_) @qualifier property: (property_identifier) @name)) @reference.call
(jsx_expression (identifier) @name @reference.value)
; `<ThemeContext.Provider>` provides the context: a use of `ThemeContext`.
(jsx_opening_element
  name: (member_expression object: (identifier) @name property: (property_identifier) @_member)
  (#any-of? @_member "Provider" "Consumer")) @reference.value
