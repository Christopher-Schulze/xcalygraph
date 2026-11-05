//! Java: `public`/`protected` visibility, JUnit tests, `static main` entry, annotation-registered
//! framework methods as entrypoints. The import and annotation helpers serve every JVM language.

#[cfg(feature = "lang-java")]
use super::Pack;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext};
#[cfg(feature = "lang-java")]
use crate::extract::engine::{Hooks, ImportExpansion};

#[cfg(feature = "lang-java")]
pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Java,
    language: |_| tree_sitter_java::LANGUAGE.into(),
    query: include_str!("../queries/java.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import: ImportExpansion::Text(jvm_import),
        builtin,
    },
};

/// Text of a definition before its name as a whole word: modifiers, annotations, return type.
pub(super) fn header<'a>(definition: &Definition<'a>) -> &'a str {
    let text = definition.text;
    let word =
        |ch: Option<char>| ch.is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '@');
    text.match_indices(definition.name)
        .find(|(start, name)| {
            !word(text[..*start].chars().next_back())
                && !word(text[start + name.len()..].chars().next())
        })
        .map_or(text, |(start, _)| &text[..start])
}

#[cfg(feature = "lang-java")]
fn exported(definition: &Definition) -> bool {
    let modifiers = header(definition);
    modifiers
        .split_whitespace()
        .any(|word| matches!(word, "public" | "protected"))
        || matches!(definition.parent, Some((SymKind::Iface, _)))
        || (definition.kind == SymKind::Const
            && matches!(definition.parent, Some((SymKind::Enum, true))))
}

/// Annotation names in a definition header (`@Test`, `@GetMapping("/x")` -> `GetMapping`).
pub(super) fn annotations(header: &str) -> impl Iterator<Item = &str> {
    header.split('@').skip(1).map(|part| {
        part.split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '.'))
            .next()
            .unwrap_or_default()
            .rsplit('.')
            .next()
            .unwrap_or_default()
    })
}

/// Annotations that mark a test method in JUnit 4 and 5, TestNG, Kotest and Spock.
pub(super) fn is_test_annotation(name: &str) -> bool {
    matches!(
        name,
        "Test" | "ParameterizedTest" | "RepeatedTest" | "TestFactory" | "TestTemplate"
    )
}

/// Annotations that only document or check a method; any other annotation registers it with a
/// framework (`@Bean`, `@GetMapping`, `@Scheduled`, `@Subscribe`), which calls it.
pub(super) fn is_passive_annotation(name: &str) -> bool {
    matches!(
        name,
        "Override"
            | "Deprecated"
            | "SuppressWarnings"
            | "SafeVarargs"
            | "FunctionalInterface"
            | "Nullable"
            | "NonNull"
            | "Nonnull"
            | "NotNull"
            | "CheckReturnValue"
            | "VisibleForTesting"
            | "JvmStatic"
            | "JvmOverloads"
            | "Throws"
            | "Synchronized"
    )
}

/// Test and entry flags shared by the JVM languages: test annotations, test classes in test
/// files, `main`, framework-registered methods.
pub(super) fn jvm_flags(
    definition: &Definition,
    context: &FileContext,
    file_type: FileType,
    test_suffixes: &[&str],
    static_main: bool,
) -> u8 {
    let head = header(definition);
    let method = matches!(definition.kind, SymKind::Method | SymKind::Fn);
    let mut flags = 0;
    if method && annotations(head).any(is_test_annotation) {
        flags |= FLAG_TEST;
    }
    if definition.kind == SymKind::Class
        && crate::domain::role::is_test_path(file_type, context.path)
        && ["Test", "Tests", "IT"]
            .iter()
            .chain(test_suffixes)
            .any(|suffix| definition.name.ends_with(suffix))
    {
        flags |= FLAG_TEST;
    }
    if method && definition.name == "main" && (!static_main || head.contains("static")) {
        flags |= FLAG_ENTRY;
    }
    let registrable = method || matches!(definition.kind, SymKind::Class | SymKind::Fn);
    if registrable
        && annotations(head).any(|name| !is_passive_annotation(name) && !is_test_annotation(name))
    {
        flags |= FLAG_ENTRY;
    }
    flags
}

#[cfg(feature = "lang-java")]
fn flags(definition: &Definition, context: &FileContext) -> u8 {
    jvm_flags(definition, context, FileType::Java, &[], true)
}

/// JVM import statements (Java, Kotlin, Scala, Groovy): `import a.b.C` -> spec `a.b`, name `C`;
/// `import a.b.*` (Scala `a.b._`) -> name `*`; `import a.b.C as D` (Scala `a.b.{C => D}`) ->
/// alias `D`; `import static a.b.C.m` -> spec `static:a.b.C`, name `m`; Scala `import a.B, c.D`
/// -> one import per path.
pub(super) fn jvm_import(statement: &str, line: u32, _reexport: bool) -> Vec<Import> {
    let text = statement
        .trim()
        .trim_start_matches("import")
        .trim()
        .trim_end_matches(';')
        .trim();
    let mut depth = 0;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .flat_map(|part| import_path(part.trim(), line))
        .collect()
}

/// One import path of [`jvm_import`].
fn import_path(text: &str, line: u32) -> Vec<Import> {
    let (is_static, text) = match text.strip_prefix("static ") {
        Some(rest) => (true, rest.trim()),
        None => (false, text),
    };
    let one = |path: &str, alias: Option<&str>| -> Option<Import> {
        let path: String = path.split_whitespace().collect();
        let (spec, name) = path.rsplit_once('.')?;
        let name = if name == "_" { "*" } else { name };
        Some(Import {
            spec: if is_static {
                format!("static:{spec}")
            } else {
                spec.to_owned()
            },
            alias: alias.map(str::to_owned).filter(|alias| alias != "_"),
            names: vec![name.to_owned()],
            line,
            reexport: false,
        })
    };
    // Scala selectors: `a.b.{C, D => E, _}`.
    if let Some((base, selectors)) = text.split_once('{') {
        let base = base.trim().trim_end_matches('.');
        return selectors
            .trim_end_matches('}')
            .split(',')
            .filter_map(|selector| {
                let (name, alias) = match selector
                    .split_once("=>")
                    .or_else(|| selector.split_once(" as "))
                {
                    Some((name, alias)) => (name.trim(), Some(alias.trim())),
                    None => (selector.trim(), None),
                };
                one(&format!("{base}.{name}"), alias)
            })
            .collect();
    }
    let (path, alias) = match text.split_once(" as ") {
        Some((path, alias)) => (path, Some(alias.trim())),
        None => (text, None),
    };
    one(path, alias).into_iter().collect()
}

/// `java.lang` types, visible everywhere without import.
#[cfg(feature = "lang-java")]
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Object"
            | "Integer"
            | "Long"
            | "Short"
            | "Byte"
            | "Double"
            | "Float"
            | "Boolean"
            | "Character"
            | "Number"
            | "Void"
            | "Math"
            | "System"
            | "Thread"
            | "Runnable"
            | "Class"
            | "Enum"
            | "Record"
            | "Iterable"
            | "Comparable"
            | "CharSequence"
            | "StringBuilder"
            | "Throwable"
            | "Exception"
            | "Error"
            | "RuntimeException"
            | "IllegalArgumentException"
            | "IllegalStateException"
            | "NullPointerException"
            | "UnsupportedOperationException"
            | "IndexOutOfBoundsException"
            | "InterruptedException"
            | "Override"
            | "Deprecated"
            | "SuppressWarnings"
            | "FunctionalInterface"
            | "AutoCloseable"
            | "var"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jvm_imports_cover_single_wildcard_static_alias_and_selectors() {
        let specs = |text: &str| -> Vec<(String, Vec<String>, Option<String>)> {
            jvm_import(text, 1, false)
                .into_iter()
                .map(|import| (import.spec, import.names, import.alias))
                .collect()
        };
        assert_eq!(
            specs("import a.b.C;"),
            [("a.b".to_owned(), vec!["C".to_owned()], None)]
        );
        assert_eq!(
            specs("import a.b.*;"),
            [("a.b".to_owned(), vec!["*".to_owned()], None)]
        );
        assert_eq!(
            specs("import static a.b.C.m;"),
            [("static:a.b.C".to_owned(), vec!["m".to_owned()], None)]
        );
        assert_eq!(
            specs("import a.b.C as D"),
            [("a.b".to_owned(), vec!["C".to_owned()], Some("D".to_owned()))]
        );
        assert_eq!(
            specs("import a.B, c.{D as E}"),
            [
                ("a".to_owned(), vec!["B".to_owned()], None),
                ("c".to_owned(), vec!["D".to_owned()], Some("E".to_owned())),
            ]
        );
        assert_eq!(
            specs("import a.b.{C, D => E, _}"),
            [
                ("a.b".to_owned(), vec!["C".to_owned()], None),
                ("a.b".to_owned(), vec!["D".to_owned()], Some("E".to_owned())),
                ("a.b".to_owned(), vec!["*".to_owned()], None),
            ]
        );
    }
}
