//! C#: `public`, `protected` and `internal` visibility, xUnit, NUnit and MSTest tests,
//! `static Main` entry, framework attributes (`[HttpGet]`, `[ApiController]`) as entrypoints. The
//! `using` and attribute helpers serve VB.NET too.

use super::Pack;
use super::java::header;
use crate::domain::facts::{FLAG_ENTRY, FLAG_TEST, Import, SymKind};
use crate::domain::filetype::FileType;
use crate::extract::engine::{Definition, FileContext, Hooks};

#[cfg(feature = "lang-csharp")]
pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::CSharp,
    language: |_| tree_sitter_c_sharp::LANGUAGE.into(),
    query: include_str!("../queries/csharp.scm"),
    hooks: Hooks {
        exported,
        signature_prefix: |_, _| "",
        flags,
        expand_import: using_import,
        builtin,
    },
};

fn exported(definition: &Definition) -> bool {
    header(definition)
        .split_whitespace()
        .any(|word| matches!(word, "public" | "protected" | "internal"))
        || matches!(definition.parent, Some((SymKind::Iface, _)))
        || (definition.kind == SymKind::Const
            && matches!(definition.parent, Some((SymKind::Enum, true))))
}

/// Attribute names in a definition header: `[Fact]`, `[HttpGet("x"), Authorize]` (C#) and
/// `<Fact>` (VB.NET), with an `Attribute` suffix removed.
pub(super) fn attributes(header: &str, open: char, close: char) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = header;
    while let Some(start) = rest.find(open) {
        let Some(end) = rest[start..].find(close) else {
            break;
        };
        for part in rest[start + 1..start + end].split(',') {
            let name = part
                .trim()
                .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == ':'))
                .next()
                .unwrap_or_default();
            let name = name.rsplit(['.', ':']).next().unwrap_or_default();
            let name = name.strip_suffix("Attribute").unwrap_or(name);
            if !name.is_empty() {
                names.push(name);
            }
        }
        rest = &rest[start + end + 1..];
    }
    names
}

/// Test and entry flags shared by C# and VB.NET.
pub(super) fn dotnet_flags(definition: &Definition, attributes: &[&str], main: bool) -> u8 {
    let method = matches!(definition.kind, SymKind::Method | SymKind::Fn);
    let mut flags = 0;
    let is_test = |name: &&str| {
        matches!(
            *name,
            "Fact"
                | "Theory"
                | "Test"
                | "TestCase"
                | "TestCaseSource"
                | "TestMethod"
                | "DataTestMethod"
        )
    };
    if method && attributes.iter().any(is_test) {
        flags |= FLAG_TEST;
    }
    if definition.kind == SymKind::Class
        && attributes
            .iter()
            .any(|name| matches!(*name, "TestFixture" | "TestClass"))
    {
        flags |= FLAG_TEST;
    }
    if method && definition.name == "Main" && main {
        flags |= FLAG_ENTRY;
    }
    let registrable = method || definition.kind == SymKind::Class;
    if registrable
        && attributes
            .iter()
            .any(|name| !is_test(name) && !is_passive_attribute(name))
    {
        flags |= FLAG_ENTRY;
    }
    flags
}

/// Attributes that only document or tune a definition; any other registers it with a framework.
fn is_passive_attribute(name: &str) -> bool {
    matches!(
        name,
        "Obsolete"
            | "Serializable"
            | "NonSerialized"
            | "Flags"
            | "DebuggerStepThrough"
            | "DebuggerDisplay"
            | "DebuggerHidden"
            | "DebuggerNonUserCode"
            | "MethodImpl"
            | "Pure"
            | "NotNull"
            | "MaybeNull"
            | "AllowNull"
            | "DisallowNull"
            | "NotNullWhen"
            | "SuppressMessage"
            | "ExcludeFromCodeCoverage"
            | "EditorBrowsable"
            | "Description"
            | "DefaultValue"
            | "CLSCompliant"
            | "ComVisible"
            | "GeneratedCode"
            | "CompilerGenerated"
            | "AggressiveInlining"
            | "SkipLocalsInit"
            | "Required"
    )
}

fn flags(definition: &Definition, _context: &FileContext) -> u8 {
    let head = header(definition);
    let main = head.split_whitespace().any(|word| word == "static");
    dotnet_flags(definition, &attributes(head, '[', ']'), main)
}

/// `using A.B;` -> namespace `A.B` wholly visible (name `*`); `using X = A.B.C;` -> alias `X` for
/// `C` of `A.B`; `using static A.B.C;` -> spec `static:A.B.C`; `global using` marks the import
/// `reexport` (visible in every file of the root).
pub(super) fn using_import(statement: &str, line: u32, _reexport: bool) -> Vec<Import> {
    let mut text = statement.trim().trim_end_matches(';').trim();
    let global = text.starts_with("global ");
    text = text
        .trim_start_matches("global ")
        .trim()
        .trim_start_matches("using")
        .trim();
    let is_static = text.starts_with("static ");
    text = text.trim_start_matches("static ").trim();
    let import = |spec: String, name: &str, alias: Option<&str>| Import {
        spec,
        alias: alias.map(str::to_owned),
        names: vec![name.to_owned()],
        line,
        reexport: global,
    };
    if let Some((alias, target)) = text.split_once('=') {
        let target: String = target.split_whitespace().collect();
        let target = target.split('<').next().unwrap_or_default().to_owned();
        return match target.rsplit_once('.') {
            Some((spec, name)) => vec![import(spec.to_owned(), name, Some(alias.trim()))],
            None => vec![import(String::new(), &target, Some(alias.trim()))],
        };
    }
    let spec: String = text.split_whitespace().collect();
    if is_static {
        return vec![import(format!("static:{spec}"), "*", None)];
    }
    vec![import(spec, "*", None)]
}

/// Keywords and `System` names visible everywhere (implicit usings).
fn builtin(name: &str) -> bool {
    matches!(
        name,
        "var"
            | "dynamic"
            | "nameof"
            | "Console"
            | "Math"
            | "String"
            | "Object"
            | "Exception"
            | "ArgumentException"
            | "ArgumentNullException"
            | "InvalidOperationException"
            | "NotImplementedException"
            | "NotSupportedException"
            | "Task"
            | "ValueTask"
            | "List"
            | "Dictionary"
            | "HashSet"
            | "IEnumerable"
            | "IList"
            | "IDictionary"
            | "ICollection"
            | "IReadOnlyList"
            | "IReadOnlyDictionary"
            | "Func"
            | "Action"
            | "Guid"
            | "DateTime"
            | "TimeSpan"
            | "CancellationToken"
            | "Environment"
            | "Obsolete"
            | "Serializable"
            | "Flags"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usings_cover_namespace_alias_static_and_global() {
        let specs = |text: &str| -> Vec<(String, Vec<String>, Option<String>, bool)> {
            using_import(text, 1, false)
                .into_iter()
                .map(|import| (import.spec, import.names, import.alias, import.reexport))
                .collect()
        };
        assert_eq!(
            specs("using App.Store;"),
            [("App.Store".to_owned(), vec!["*".to_owned()], None, false)]
        );
        assert_eq!(
            specs("using Repo = App.Store.Repository;"),
            [(
                "App.Store".to_owned(),
                vec!["Repository".to_owned()],
                Some("Repo".to_owned()),
                false
            )]
        );
        assert_eq!(
            specs("using static App.Limits;"),
            [(
                "static:App.Limits".to_owned(),
                vec!["*".to_owned()],
                None,
                false
            )]
        );
        assert_eq!(
            specs("global using App.Core;"),
            [("App.Core".to_owned(), vec!["*".to_owned()], None, true)]
        );
        assert_eq!(
            attributes("[Fact, Trait(\"a\", \"b\")] public void X()", '[', ']'),
            ["Fact", "Trait"]
        );
        assert_eq!(
            attributes("[HttpGetAttribute(\"x\")]", '[', ']'),
            ["HttpGet"]
        );
    }
}
