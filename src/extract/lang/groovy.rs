//! Groovy (`.groovy`, `.gradle`): public unless `private`, JUnit tests and Spock specifications
//! (`*Spec`), `main` and scripts as entrypoints; imports and annotations as in Java.

use super::Pack;
use super::java::{header, jvm_flags, jvm_import};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks, ImportExpansion};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Groovy,
    language: |_| tree_sitter_groovy::LANGUAGE.into(),
    query: include_str!("../queries/groovy.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(jvm_import),
        builtin,
    },
};

fn exported(definition: &Definition) -> bool {
    !header(definition)
        .split_whitespace()
        .any(|word| word == "private")
}

fn flags(definition: &Definition, context: &FileContext) -> u8 {
    jvm_flags(definition, context, FileType::Groovy, &["Spec"], false)
}

/// `java.lang`, `groovy.lang` and GDK names visible without import.
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Object"
            | "Integer"
            | "Long"
            | "Double"
            | "Boolean"
            | "Map"
            | "List"
            | "Closure"
            | "GString"
            | "System"
            | "Math"
            | "Exception"
            | "RuntimeException"
            | "IllegalArgumentException"
            | "IllegalStateException"
            | "def"
            | "var"
            | "println"
            | "print"
            | "Override"
    )
}

/// Groovy ends statements at line breaks, the grammar expects `;`: appends one to every line
/// that completes a statement, keeping all line numbers. A line continues (no `;`) when it ends
/// with an operator, comma or opening bracket, when the next code line starts with an operator,
/// `.`, `{` or a closing bracket, inside triple-quoted strings and block comments, and after
/// annotation-only lines.
pub fn terminate_statements(source: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(source);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = String::with_capacity(source.len() + lines.len());
    let mut in_block = false;
    for (index, line) in lines.iter().enumerate() {
        out.push_str(line);
        let opens = line.matches("\"\"\"").count() + line.matches("'''").count();
        let code = strip_line_comment(line).trim_end();
        let was_block = in_block;
        if code.contains("/*") && !code.contains("*/") {
            in_block = true;
        } else if in_block && code.contains("*/") {
            in_block = false;
        }
        let next = lines[index + 1..]
            .iter()
            .map(|next| next.trim())
            .find(|next| !next.is_empty() && !next.starts_with("//"))
            .unwrap_or("}");
        let completes = code.chars().next_back().is_some_and(|last| {
            last.is_alphanumeric() || matches!(last, '_' | ')' | ']' | '"' | '\'' | '}')
        });
        let continues = next.starts_with([
            '.', '?', '&', '|', '+', '-', '*', '/', ':', '{', ')', ']', '=', '<', '>',
        ]) || next.starts_with("as ");
        let annotation = code.trim_start().starts_with('@') && !code.contains(' ');
        if completes
            && !continues
            && !annotation
            && !was_block
            && !in_block
            && opens % 2 == 0
            && !code.ends_with('}')
        {
            out.push(';');
        }
        if index + 1 < lines.len() {
            out.push('\n');
        }
    }
    out.into_bytes()
}

/// A line without its `//` comment (not inside a string literal).
fn strip_line_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    for (index, ch) in line.char_indices() {
        match quote {
            Some(open) if ch == open && previous != '\\' => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '/' && previous == '/' => return &line[..index - 1],
            None => {}
        }
        previous = ch;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statements_end_at_line_breaks_unless_continued() {
        let source = "@Field\nprivate final Formatter formatter\nGreeter(Formatter f) {\n  this.f = f\n  def x = a\n    .b()\n  call(1,\n    2)\n}\n";
        let want = "@Field\nprivate final Formatter formatter;\nGreeter(Formatter f) {\n  this.f = f;\n  def x = a\n    .b();\n  call(1,\n    2);\n}\n";
        assert_eq!(
            String::from_utf8_lossy(&terminate_statements(source.as_bytes())),
            want
        );
    }
}
