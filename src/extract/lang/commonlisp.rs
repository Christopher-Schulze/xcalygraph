//! Common Lisp: `in-package` names the module; `defgeneric` declares what `defmethod`s define;
//! special operators and standard functions are builtins.

use super::Pack;
use crate::domain::facts::FLAG_DECLARATION;
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Definition, FileContext, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Lisp,
    language: |_| tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into(),
    query: include_str!("../queries/commonlisp.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(engine::single_import),
        builtin,
    },
};

fn flags(definition: &Definition, _context: &FileContext) -> u8 {
    let generic = definition
        .text
        .trim_start_matches('(')
        .get(..10)
        .is_some_and(|head| head.eq_ignore_ascii_case("defgeneric"));
    if generic { FLAG_DECLARATION } else { 0 }
}

fn builtin(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("def")
        || lower.starts_with("with-")
        || matches!(
            lower.as_str(),
            "let"
                | "let*"
                | "if"
                | "when"
                | "unless"
                | "cond"
                | "case"
                | "ecase"
                | "typecase"
                | "progn"
                | "prog1"
                | "setf"
                | "setq"
                | "lambda"
                | "loop"
                | "dolist"
                | "dotimes"
                | "do"
                | "format"
                | "list"
                | "cons"
                | "car"
                | "cdr"
                | "first"
                | "second"
                | "rest"
                | "last"
                | "append"
                | "mapcar"
                | "mapc"
                | "funcall"
                | "apply"
                | "eq"
                | "eql"
                | "equal"
                | "not"
                | "and"
                | "or"
                | "+"
                | "-"
                | "*"
                | "/"
                | "="
                | "<"
                | ">"
                | "<="
                | ">="
                | "declare"
                | "return"
                | "return-from"
                | "push"
                | "pop"
                | "nth"
                | "length"
                | "gethash"
                | "make-hash-table"
                | "slot-value"
                | "make-instance"
                | "error"
                | "values"
                | "multiple-value-bind"
                | "destructuring-bind"
                | "handler-case"
                | "unwind-protect"
                | "flet"
                | "labels"
                | "incf"
                | "decf"
                | "quote"
                | "function"
                | "assert"
                | "typep"
                | "null"
                | "member"
                | "assoc"
                | "find"
                | "remove"
                | "concatenate"
                | "string"
                | "intern"
                | "in-package"
                | "export"
                | "ignore-errors"
                | "block"
                | "tagbody"
                | "go"
                | "the"
                | "check-type"
                | "print"
                | "princ"
                | "terpri"
                | "vector"
                | "aref"
                | "elt"
                | "subseq"
                | "reverse"
                | "sort"
                | "reduce"
                | "zerop"
                | "1+"
                | "1-"
                | "car*"
        )
}
