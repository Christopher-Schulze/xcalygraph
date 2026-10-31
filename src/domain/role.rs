//! File roles and their classification (`docs/spec.md` section 3, Corpus model).

use super::filetype::{self, Class, Detection, FileType};

pub const LARGE_LIMIT: u64 = 8 * 1024 * 1024;
pub const LARGE_DOCUMENT_LIMIT: u64 = 64 * 1024 * 1024;
/// Bytes of content inspected for markers, generated headers and NUL bytes.
pub const HEAD_LEN: usize = 8 * 1024;
const GENERATED_HEAD_LEN: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Role {
    Src = 1,
    Test,
    Gen,
    Fixture,
    Manifest,
    Config,
    Doc,
    Asset,
    Sensitive,
    Large,
    Binary,
    Other,
}

pub const ROLES: [Role; 12] = [
    Role::Src,
    Role::Test,
    Role::Gen,
    Role::Fixture,
    Role::Manifest,
    Role::Config,
    Role::Doc,
    Role::Asset,
    Role::Sensitive,
    Role::Large,
    Role::Binary,
    Role::Other,
];

impl Role {
    pub fn from_u8(value: u8) -> Option<Self> {
        ROLES.into_iter().find(|role| *role as u8 == value)
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Src => "src",
            Self::Test => "test",
            Self::Gen => "gen",
            Self::Fixture => "fixture",
            Self::Manifest => "manifest",
            Self::Config => "config",
            Self::Doc => "doc",
            Self::Asset => "asset",
            Self::Sensitive => "sensitive",
            Self::Large => "large",
            Self::Binary => "binary",
            Self::Other => "other",
        }
    }

    /// Roles whose content is extracted; the rest are indexed by path only.
    pub const fn is_extracted(self) -> bool {
        matches!(
            self,
            Self::Src
                | Self::Test
                | Self::Gen
                | Self::Manifest
                | Self::Config
                | Self::Doc
                | Self::Asset
        )
    }

    /// Ranking weight of `find` and `ctx` (section 7.2).
    pub const fn weight(self) -> f32 {
        match self {
            Self::Src => 1.0,
            Self::Test => 0.3,
            Self::Gen => 0.2,
            Self::Doc | Self::Config | Self::Manifest => 0.5,
            Self::Asset => 0.3,
            _ => 0.1,
        }
    }
}

/// What the classifier needs to know about one file.
pub struct FileFacts<'a> {
    /// Root-relative path with `/` separators.
    pub path: &'a str,
    pub size: u64,
    /// First bytes of the content (up to [`HEAD_LEN`]); `None` when content may not be read.
    pub head: Option<&'a [u8]>,
    /// True when a C++ source with the same stem exists in the same directory.
    pub cpp_sibling: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    pub role: Role,
    pub file_type: Option<FileType>,
}

/// True when content of the path must never be read (role `sensitive`).
pub fn is_sensitive(path: &str) -> bool {
    let name = file_name(path);
    let segments = || path.split('/');
    (name.starts_with(".env") && name != ".env.example")
        || [".pem", ".key", ".p12"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || segments()
            .rev()
            .skip(1)
            .any(|segment| matches!(segment, ".ssh" | ".aws" | ".gnupg"))
}

/// Size limit above which the file is indexed by path only.
pub fn large_limit(path: &str) -> u64 {
    match filetype::detect(file_name(path)) {
        Detection::Known(FileType::Pdf | FileType::Docx | FileType::Xlsx) => LARGE_DOCUMENT_LIMIT,
        _ => LARGE_LIMIT,
    }
}

/// True when the content is needed to classify or index the file.
pub fn needs_content(path: &str, size: u64) -> bool {
    !is_sensitive(path) && size <= large_limit(path)
}

/// Assigns the role and file type; first match in the role precedence wins.
pub fn classify(file: &FileFacts) -> Classification {
    let path = file.path;
    let name = file_name(path);
    let detected = detect_type(file);
    let with = |role| Classification {
        role,
        file_type: detected,
    };
    if is_sensitive(path) {
        return Classification {
            role: Role::Sensitive,
            file_type: None,
        };
    }
    if is_fixture(path) {
        return with(Role::Fixture);
    }
    if file.size > large_limit(path) {
        return with(Role::Large);
    }
    let Some(head) = file.head else {
        return with(Role::Other);
    };
    if is_manifest(name) {
        return with(Role::Manifest);
    }
    let Some(file_type) = detected else {
        let role = if head.contains(&0) {
            Role::Binary
        } else {
            Role::Other
        };
        return with(role);
    };
    if file_type.is_code() {
        if is_generated(head) {
            return with(Role::Gen);
        }
        if is_test_path(file_type, path) {
            return with(Role::Test);
        }
        return with(Role::Src);
    }
    let role = match file_type.class() {
        Class::Config => Role::Config,
        Class::Doc => Role::Doc,
        Class::Asset => Role::Asset,
        Class::Deep | Class::Structural => Role::Src,
    };
    with(role)
}

fn detect_type(file: &FileFacts) -> Option<FileType> {
    let head = file.head.unwrap_or_default();
    match filetype::detect(file_name(file.path)) {
        Detection::Known(file_type) => Some(file_type),
        Detection::Ambiguous(ambiguous) => {
            filetype::resolve_ambiguous(ambiguous, head, file.cpp_sibling)
        }
        Detection::Shebang => filetype::shebang_type(head),
        Detection::Unknown => None,
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn directories(path: &str) -> impl Iterator<Item = &str> {
    let mut segments: Vec<&str> = path.split('/').collect();
    segments.pop();
    segments.into_iter()
}

fn is_fixture(path: &str) -> bool {
    directories(path).any(|segment| {
        matches!(
            segment,
            "testdata" | "fixtures" | "__fixtures__" | "__snapshots__"
        )
    })
}

fn is_manifest(name: &str) -> bool {
    const NAMES: [&str; 23] = [
        "go.mod",
        "go.work",
        "Cargo.toml",
        "package.json",
        "pnpm-workspace.yaml",
        "pyproject.toml",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
        "Gemfile",
        "composer.json",
        "Package.swift",
        "pubspec.yaml",
        "mix.exs",
        "rebar.config",
        "Project.toml",
        "DESCRIPTION",
        "dune-project",
        "build.zig",
        "build.zig.zon",
        "foundry.toml",
    ];
    const SUFFIXES: [&str; 6] = [".csproj", ".fsproj", ".vbproj", ".sln", ".slnx", ".gemspec"];
    NAMES.contains(&name)
        || matches!(
            name,
            "remappings.txt"
                | "CMakeLists.txt"
                | "compile_commands.json"
                | "tsconfig.json"
                | "jsconfig.json"
        )
        || SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
        || (name.starts_with("requirements") && name.ends_with(".txt"))
}

fn is_generated(head: &[u8]) -> bool {
    let head = &head[..head.len().min(GENERATED_HEAD_LEN)];
    let text = String::from_utf8_lossy(head);
    // Go convention, `@generated`, and generator banners such as "Generated by the gRPC Python
    // protocol compiler plugin. DO NOT EDIT!" or "auto-generated ... do not edit".
    let lower = text.to_ascii_lowercase();
    (lower.contains("generated") && lower.contains("do not edit")) || text.contains("@generated")
}

/// File-level test conventions of the language table; symbol-level conventions (test functions,
/// attributes) are flags set by extraction.
pub fn is_test_path(file_type: FileType, path: &str) -> bool {
    use FileType::*;
    let name = file_name(path);
    let stem = name.split('.').next().unwrap_or(name);
    let has_dir = |wanted: &[&str]| directories(path).any(|segment| wanted.contains(&segment));
    match file_type {
        Go => name.ends_with("_test.go"),
        Rust => has_dir(&["tests"]),
        TypeScript | JavaScript | Vue | Svelte | Astro => {
            name.contains(".test.") || name.contains(".spec.") || has_dir(&["__tests__"])
        }
        Python => {
            (name.starts_with("test_") && name.ends_with(".py"))
                || name.ends_with("_test.py")
                || has_dir(&["tests"])
        }
        Java | Kotlin => jvm_test(path, stem, &[]),
        Scala => jvm_test(path, stem, &["Spec", "Suite"]),
        Groovy => jvm_test(path, stem, &["Spec"]),
        C | Cpp => {
            stem.ends_with("_test")
                || stem.starts_with("test_")
                || stem.ends_with("_unittest")
                || has_dir(&["test", "tests"])
        }
        ObjC => name.ends_with("Tests.m") || name.ends_with("Tests.mm"),
        Swift => has_dir(&["Tests"]) || name.ends_with("Tests.swift"),
        Ruby => {
            name.ends_with("_spec.rb")
                || name.ends_with("_test.rb")
                || (name.starts_with("test_") && name.ends_with(".rb"))
                || has_dir(&["spec", "test"])
        }
        // .NET test projects: `X.Tests/`, `X.UnitTests/`, `tests/`; test classes `*Tests.cs`.
        CSharp | VbNet => {
            directories(path).any(|segment| {
                segment.ends_with(".Tests")
                    || segment.ends_with(".UnitTests")
                    || segment.ends_with(".IntegrationTests")
            }) || has_dir(&["tests", "test", "Tests"])
                || stem.ends_with("Tests")
                || stem.ends_with("Test")
        }
        Php => name.ends_with("Test.php") || has_dir(&["tests"]),
        Lua => name.ends_with("_spec.lua") || has_dir(&["spec"]),
        Elixir => name.ends_with("_test.exs") || has_dir(&["test"]),
        Erlang => {
            name.ends_with("_SUITE.erl") || name.ends_with("_tests.erl") || has_dir(&["test"])
        }
        Dart => name.ends_with("_test.dart") || has_dir(&["test"]),
        Julia => has_dir(&["test"]),
        R => path.contains("tests/testthat/") && name.starts_with("test-"),
        OCaml | Fortran => has_dir(&["test", "tests"]),
        Lisp => has_dir(&["test", "tests", "t"]),
        Solidity => name.ends_with(".t.sol") || has_dir(&["test"]),
        Verilog => stem.ends_with("_tb") || stem.starts_with("tb_"),
        PowerShell => name.ends_with(".Tests.ps1"),
        Bash => name.ends_with(".bats"),
        Terraform => name.ends_with(".tftest.hcl"),
        _ => false,
    }
}

fn jvm_test(path: &str, stem: &str, extra_suffixes: &[&str]) -> bool {
    path.starts_with("src/test/")
        || path.contains("/src/test/")
        || ["Test", "Tests", "IT"]
            .iter()
            .chain(extra_suffixes)
            .any(|suffix| stem.ends_with(suffix) && stem.len() > suffix.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_text(path: &str, content: &str) -> Classification {
        classify(&FileFacts {
            path,
            size: content.len() as u64,
            head: Some(content.as_bytes()),
            cpp_sibling: false,
        })
    }

    fn role(path: &str, content: &str) -> Role {
        classify_text(path, content).role
    }

    #[test]
    fn every_role_is_assigned() {
        let cases = [
            ("cmd/main.go", "package main", Role::Src),
            ("cmd/main_test.go", "package main", Role::Test),
            (
                "api/types.pb.go",
                "// Code generated by protoc. DO NOT EDIT.",
                Role::Gen,
            ),
            ("pkg/testdata/sample.go", "package x", Role::Fixture),
            ("go.mod", "module example.com/x", Role::Manifest),
            ("deploy/compose.yaml", "services: {}", Role::Config),
            ("docs/guide.md", "# Guide", Role::Doc),
            ("assets/logo.svg", "<svg/>", Role::Asset),
            (".env", "SECRET=1", Role::Sensitive),
            ("blob.bin", "\0\0\u{1}", Role::Binary),
            ("LICENSE", "MIT License", Role::Other),
        ];
        for (path, content, want) in cases {
            assert_eq!(role(path, content), want, "{path}");
        }
        let large = classify(&FileFacts {
            path: "data/big.json",
            size: LARGE_LIMIT + 1,
            head: None,
            cpp_sibling: false,
        });
        assert_eq!(large.role, Role::Large);
        assert_eq!(Role::from_u8(Role::Other as u8), Some(Role::Other));
    }

    #[test]
    fn precedence_puts_sensitive_and_fixture_first() {
        assert_eq!(role("testdata/.env.local", "A=1"), Role::Sensitive);
        assert_eq!(role("home/.ssh/config", "Host x"), Role::Sensitive);
        assert_eq!(role("certs/server.pem", "-----BEGIN"), Role::Sensitive);
        assert_eq!(role(".env.example", "A="), Role::Config);
        assert_eq!(role("fixtures/go.mod", "module x"), Role::Fixture);
        assert_eq!(role("tests/gen_test.go", "// @generated"), Role::Gen);
        assert_eq!(
            role(
                "src/demo_pb2_grpc.py",
                "# Generated by the gRPC Python protocol compiler plugin. DO NOT EDIT!"
            ),
            Role::Gen
        );
        assert_eq!(
            role("Package.swift", "// swift-tools-version"),
            Role::Manifest
        );
    }

    #[test]
    fn documents_have_a_higher_size_limit() {
        let pdf = FileFacts {
            path: "paper.pdf",
            size: LARGE_LIMIT + 1,
            head: Some(b"%PDF-1.7"),
            cpp_sibling: false,
        };
        assert_eq!(classify(&pdf).role, Role::Doc);
        assert!(!needs_content(".env", 10));
        assert!(!needs_content("x.json", LARGE_LIMIT + 1));
        assert!(needs_content("x.pdf", LARGE_LIMIT + 1));
    }

    #[test]
    fn test_conventions_per_language() {
        let tests = [
            (FileType::Rust, "crates/core/tests/api.rs"),
            (FileType::TypeScript, "src/app.spec.ts"),
            (FileType::JavaScript, "src/__tests__/app.js"),
            (FileType::Python, "pkg/test_core.py"),
            (FileType::Java, "src/test/java/a/B.java"),
            (FileType::Java, "src/main/java/a/ParserTest.java"),
            (FileType::Scala, "core/ParserSpec.scala"),
            (FileType::Cpp, "src/parser_unittest.cc"),
            (FileType::Swift, "Tests/AppTests/AppTests.swift"),
            (FileType::Ruby, "spec/models/user_spec.rb"),
            (FileType::R, "tests/testthat/test-model.R"),
            (FileType::Solidity, "src/Token.t.sol"),
            (FileType::Verilog, "rtl/alu_tb.sv"),
            (FileType::PowerShell, "Module.Tests.ps1"),
            (FileType::Terraform, "main.tftest.hcl"),
        ];
        for (file_type, path) in tests {
            assert!(is_test_path(file_type, path), "{path}");
        }
        let sources = [
            (FileType::Rust, "src/tests.rs"),
            (FileType::Java, "src/main/java/a/Test.java"),
            (FileType::Python, "pkg/testing.py"),
            (FileType::Go, "pkg/test.go"),
        ];
        for (file_type, path) in sources {
            assert!(!is_test_path(file_type, path), "{path}");
        }
    }

    #[test]
    fn shebang_and_markers_select_languages() {
        assert_eq!(
            classify_text("bin/deploy", "#!/usr/bin/env bash\nset -e").file_type,
            Some(FileType::Bash)
        );
        assert_eq!(classify_text("bin/tool", "plain").file_type, None);
        assert_eq!(
            classify_text("benchsuite", "#!/usr/bin/env python3\nimport os").file_type,
            Some(FileType::Python)
        );
        assert_eq!(
            classify_text("bin/cli", "#!/usr/bin/env node\nrequire('x')").file_type,
            Some(FileType::JavaScript)
        );
        assert_eq!(
            classify_text("src/Foo.m", "@implementation Foo").role,
            Role::Src
        );
        assert_eq!(classify_text("src/plot.m", "x = 1;").role, Role::Other);
    }
}
