; Common Lisp facts for the extraction engine (src/extract/engine.rs capture convention).

(defun (defun_header keyword: (defun_keyword) @_defun function_name: [(sym_lit) (package_lit)] @name)
  (#match? @_defun "^(?i)(defun|defgeneric|defmethod)$")) @definition.function
(defun (defun_header keyword: (defun_keyword) @_defmacro function_name: [(sym_lit) (package_lit)] @name)
  (#match? @_defmacro "^(?i)defmacro$")) @definition.macro
(list_lit . (sym_lit) @_defclass . (sym_lit) @name (#match? @_defclass "^(?i)(defclass|define-condition)$")) @definition.class
(list_lit . (sym_lit) @_defclass . (sym_lit) . (list_lit (sym_lit) @name) (#match? @_defclass "^(?i)(defclass|define-condition)$")) @reference.inherit
(list_lit . (sym_lit) @_defstruct . [(sym_lit) @name (list_lit . (sym_lit) @name)] (#match? @_defstruct "^(?i)defstruct$")) @definition.struct
(list_lit . (sym_lit) @_defvar . (sym_lit) @name (#match? @_defvar "^(?i)(defvar|defparameter|defconstant)$")) @definition.variable

(list_lit . (sym_lit) @_in . [(kwd_lit (kwd_symbol)) (sym_lit) (str_lit)] @package (#match? @_in "^(?i)in-package$"))
; `(:use :cl :util)` in `defpackage`; `(:file "util")` components of ASDF systems.
(list_lit
  . (sym_lit) @_defpackage
  . [(kwd_lit (kwd_symbol)) (sym_lit) (str_lit)] @import.alias
  (list_lit . (kwd_lit) @_use (kwd_lit (kwd_symbol) @import.path) @import)
  (#match? @_defpackage "^(?i)(defpackage|uiop:define-package)$")
  (#match? @_use "^(?i):use$"))
(list_lit . (kwd_lit) @_file . (str_lit) @import.path (#match? @_file "^(?i):file$")) @import

(list_lit . (sym_lit) @name) @reference.call
(list_lit . (package_lit package: (sym_lit) @qualifier symbol: (sym_lit) @name)) @reference.call

(list_lit
  . (package_lit package: (sym_lit) @_uiop symbol: (sym_lit) @_getenv)
  . (str_lit) @env.name
  (#match? @_uiop "^(?i)(uiop|sb-ext|sb-posix)$")
  (#match? @_getenv "^(?i)(getenv|posix-getenv)$")) @env
