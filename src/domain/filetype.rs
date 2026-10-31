//! File types: languages and formats with their ids, depth and suffixes (`docs/spec.md` section 3).

/// Every language and non-code format the corpus model knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum FileType {
    Go = 1,
    Rust,
    TypeScript,
    JavaScript,
    Vue,
    Svelte,
    Astro,
    Python,
    Java,
    Kotlin,
    Scala,
    Groovy,
    CSharp,
    VbNet,
    C,
    Cpp,
    ObjC,
    Swift,
    Ruby,
    Php,
    Lua,
    Zig,
    Elixir,
    Erlang,
    Dart,
    Julia,
    R,
    OCaml,
    Fortran,
    Pascal,
    Lisp,
    Solidity,
    Verilog,
    PowerShell,
    Bash,
    Sql,
    Terraform,
    Robot,
    Dm,
    Apex,
    Razor,
    Blade,
    Ejs,
    Jinja,
    Cobol,
    Xaml,
    DmResource,
    Json,
    Yaml,
    Toml,
    Xml,
    Ini,
    Properties,
    Proto,
    GraphQl,
    Dockerfile,
    Make,
    EnvExample,
    Markdown,
    Text,
    Rst,
    Org,
    AsciiDoc,
    Tex,
    Html,
    Pdf,
    Docx,
    Xlsx,
    Image,
    Svg,
    Video,
    Audio,
}

/// What kind of content a file type holds; decides the default role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Deep,
    Structural,
    Config,
    Doc,
    Asset,
}

/// Suffixes that need content markers to pick a file type (section 3, Detection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ambiguous {
    /// `.h`: C, C++ or Objective-C.
    Header,
    /// `.m`: Objective-C, else unknown.
    M,
    /// `.v`: Verilog, else unknown.
    V,
    /// `.inc`, `.pp`: Pascal, else unknown.
    PascalInclude,
    /// `.cl`: Common Lisp, else unknown.
    Cl,
    /// `.cls`: Apex, else unknown.
    Cls,
}

/// Result of name-based detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    Known(FileType),
    Ambiguous(Ambiguous),
    /// Extensionless file: a `sh`, `bash` or `zsh` shebang makes it Bash.
    Shebang,
    Unknown,
}

/// Every file type in discriminant order (1-based).
pub const ALL: [FileType; 72] = {
    use FileType::*;
    [
        Go, Rust, TypeScript, JavaScript, Vue, Svelte, Astro, Python, Java, Kotlin, Scala, Groovy,
        CSharp, VbNet, C, Cpp, ObjC, Swift, Ruby, Php, Lua, Zig, Elixir, Erlang, Dart, Julia, R,
        OCaml, Fortran, Pascal, Lisp, Solidity, Verilog, PowerShell, Bash, Sql, Terraform, Robot,
        Dm, Apex, Razor, Blade, Ejs, Jinja, Cobol, Xaml, DmResource, Json, Yaml, Toml, Xml, Ini,
        Properties, Proto, GraphQl, Dockerfile, Make, EnvExample, Markdown, Text, Rst, Org,
        AsciiDoc, Tex, Html, Pdf, Docx, Xlsx, Image, Svg, Video, Audio,
    ]
};

impl FileType {
    pub const fn from_u8(value: u8) -> Option<Self> {
        if value == 0 || value as usize > ALL.len() {
            return None;
        }
        Some(ALL[value as usize - 1])
    }

    pub fn from_id(id: &str) -> Option<Self> {
        ALL.into_iter().find(|file_type| file_type.id() == id)
    }

    /// Stable id used in output and in the `-l` filter.
    pub const fn id(self) -> &'static str {
        use FileType::*;
        match self {
            Go => "go",
            Rust => "rust",
            TypeScript => "typescript",
            JavaScript => "javascript",
            Vue => "vue",
            Svelte => "svelte",
            Astro => "astro",
            Python => "python",
            Java => "java",
            Kotlin => "kotlin",
            Scala => "scala",
            Groovy => "groovy",
            CSharp => "csharp",
            VbNet => "vbnet",
            C => "c",
            Cpp => "cpp",
            ObjC => "objc",
            Swift => "swift",
            Ruby => "ruby",
            Php => "php",
            Lua => "lua",
            Zig => "zig",
            Elixir => "elixir",
            Erlang => "erlang",
            Dart => "dart",
            Julia => "julia",
            R => "r",
            OCaml => "ocaml",
            Fortran => "fortran",
            Pascal => "pascal",
            Lisp => "lisp",
            Solidity => "solidity",
            Verilog => "verilog",
            PowerShell => "powershell",
            Bash => "bash",
            Sql => "sql",
            Terraform => "terraform",
            Robot => "robot",
            Dm => "dm",
            Apex => "apex",
            Razor => "razor",
            Blade => "blade",
            Ejs => "ejs",
            Jinja => "jinja",
            Cobol => "cobol",
            Xaml => "xaml",
            DmResource => "dmres",
            Json => "json",
            Yaml => "yaml",
            Toml => "toml",
            Xml => "xml",
            Ini => "ini",
            Properties => "properties",
            Proto => "proto",
            GraphQl => "graphql",
            Dockerfile => "dockerfile",
            Make => "make",
            EnvExample => "env",
            Markdown => "markdown",
            Text => "text",
            Rst => "rst",
            Org => "org",
            AsciiDoc => "asciidoc",
            Tex => "tex",
            Html => "html",
            Pdf => "pdf",
            Docx => "docx",
            Xlsx => "xlsx",
            Image => "image",
            Svg => "svg",
            Video => "video",
            Audio => "audio",
        }
    }

    pub const fn class(self) -> Class {
        use FileType::*;
        match self {
            Apex | Razor | Blade | Ejs | Jinja | Cobol | Xaml | DmResource => Class::Structural,
            Json | Yaml | Toml | Xml | Ini | Properties | Proto | GraphQl | Dockerfile | Make
            | EnvExample => Class::Config,
            Markdown | Text | Rst | Org | AsciiDoc | Tex | Html | Pdf | Docx | Xlsx => Class::Doc,
            Image | Svg | Video | Audio => Class::Asset,
            _ => Class::Deep,
        }
    }

    pub const fn is_code(self) -> bool {
        matches!(self.class(), Class::Deep | Class::Structural)
    }
}

/// Detects the file type from the file name alone (lowercase suffix match, longest first).
pub fn detect(name: &str) -> Detection {
    use FileType::*;
    match name {
        "Dockerfile" | "dockerfile" | "Containerfile" => return Detection::Known(Dockerfile),
        "Makefile" | "makefile" | "GNUmakefile" => return Detection::Known(Make),
        ".env.example" => return Detection::Known(EnvExample),
        _ => {}
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".blade.php") {
        return Detection::Known(Blade);
    }
    let Some(dot) = lower.rfind('.') else {
        return Detection::Shebang;
    };
    if dot == 0 {
        return Detection::Unknown;
    }
    let extension = &lower[dot + 1..];
    let file_type = match extension {
        "go" => Go,
        "rs" => Rust,
        "ts" | "tsx" | "mts" | "cts" | "ets" => TypeScript,
        "js" | "jsx" | "mjs" | "cjs" => JavaScript,
        "vue" => Vue,
        "svelte" => Svelte,
        "astro" => Astro,
        "py" | "pyi" => Python,
        "java" => Java,
        "kt" | "kts" => Kotlin,
        "scala" => Scala,
        "groovy" | "gradle" => Groovy,
        "cs" => CSharp,
        "vb" => VbNet,
        "c" => C,
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "cu" | "cuh" | "metal" => Cpp,
        "mm" => ObjC,
        "swift" => Swift,
        "rb" | "rake" => Ruby,
        "php" => Php,
        "lua" | "luau" | "toc" => Lua,
        "zig" => Zig,
        "ex" | "exs" => Elixir,
        "erl" | "hrl" | "escript" => Erlang,
        "dart" => Dart,
        "jl" => Julia,
        "r" => R,
        "ml" | "mli" => OCaml,
        "f" | "f90" | "f95" | "f03" | "f08" => Fortran,
        "pas" | "dpr" | "dpk" | "lpr" | "dfm" | "lfm" | "lpk" => Pascal,
        "lisp" | "lsp" | "asd" => Lisp,
        "sol" => Solidity,
        "sv" | "svh" | "vh" => Verilog,
        "ps1" | "psm1" | "psd1" => PowerShell,
        "sh" | "bash" | "bats" => Bash,
        "sql" => Sql,
        "tf" | "tfvars" | "hcl" => Terraform,
        "robot" | "resource" => Robot,
        "dm" | "dme" => Dm,
        "trigger" => Apex,
        "razor" | "cshtml" => Razor,
        "ejs" => Ejs,
        "j2" | "jinja" | "jinja2" => Jinja,
        "cbl" | "cob" | "cobol" | "cpy" => Cobol,
        "xaml" => Xaml,
        "dmi" | "dmm" | "dmf" => DmResource,
        "json" | "jsonc" => Json,
        "yaml" | "yml" => Yaml,
        "toml" => Toml,
        "xml" => Xml,
        "ini" => Ini,
        "properties" => Properties,
        "proto" => Proto,
        "graphql" | "gql" => GraphQl,
        "dockerfile" => Dockerfile,
        "mk" => Make,
        "md" | "mdx" | "qmd" | "markdown" | "skill" => Markdown,
        "txt" => Text,
        "rst" => Rst,
        "org" => Org,
        "adoc" => AsciiDoc,
        "tex" => Tex,
        "html" | "htm" => Html,
        "pdf" => Pdf,
        "docx" => Docx,
        "xlsx" => Xlsx,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" => Image,
        "svg" => Svg,
        "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" => Video,
        "mp3" | "wav" | "m4a" | "ogg" => Audio,
        "h" => return Detection::Ambiguous(Ambiguous::Header),
        "m" => return Detection::Ambiguous(Ambiguous::M),
        "v" => return Detection::Ambiguous(Ambiguous::V),
        "inc" | "pp" => return Detection::Ambiguous(Ambiguous::PascalInclude),
        "cl" => return Detection::Ambiguous(Ambiguous::Cl),
        "cls" => return Detection::Ambiguous(Ambiguous::Cls),
        _ => return Detection::Unknown,
    };
    Detection::Known(file_type)
}

/// Decides an ambiguous suffix from the first 8 KiB; `cpp_sibling` is true when a C++ source
/// with the same stem exists next to a `.h` file.
pub fn resolve_ambiguous(ambiguous: Ambiguous, head: &[u8], cpp_sibling: bool) -> Option<FileType> {
    let text = String::from_utf8_lossy(head);
    let has_word = |word: &str| contains_word(&text, word);
    match ambiguous {
        Ambiguous::Header => {
            if text.contains("@interface") || text.contains("#import") {
                Some(FileType::ObjC)
            } else if has_word("class")
                || has_word("namespace")
                || has_word("template")
                || cpp_sibling
            {
                Some(FileType::Cpp)
            } else {
                Some(FileType::C)
            }
        }
        Ambiguous::M => (text.contains("#import")
            || text.contains("@interface")
            || text.contains("@implementation"))
        .then_some(FileType::ObjC),
        Ambiguous::V => (has_word("module") && has_word("endmodule")).then_some(FileType::Verilog),
        Ambiguous::PascalInclude => {
            let lower = text.to_ascii_lowercase();
            ["unit", "procedure", "function", "begin"]
                .iter()
                .any(|word| contains_word(&lower, word))
                .then_some(FileType::Pascal)
        }
        Ambiguous::Cl => text.trim_start().starts_with('(').then_some(FileType::Lisp),
        Ambiguous::Cls => {
            let lower = text.to_ascii_lowercase();
            let modifier = ["public", "private", "global", "protected"]
                .iter()
                .any(|word| contains_word(&lower, word));
            (modifier && (contains_word(&lower, "class") || contains_word(&lower, "interface")))
                .then_some(FileType::Apex)
        }
    }
}

/// Language of an extensionless script by its `#!` interpreter: shells, Python, Node and Ruby.
pub fn shebang_type(head: &[u8]) -> Option<FileType> {
    let rest = head.strip_prefix(b"#!")?;
    let line_end = rest
        .iter()
        .position(|&byte| byte == b'\n')
        .unwrap_or(rest.len());
    let line = String::from_utf8_lossy(&rest[..line_end]);
    let mut words = line.split_whitespace();
    let program = words.next()?;
    let program = program.rsplit('/').next().unwrap_or(program);
    let interpreter = if program == "env" {
        words.find(|word| !word.starts_with('-')).unwrap_or("")
    } else {
        program
    };
    let base = interpreter.trim_end_matches(|ch: char| ch.is_ascii_digit() || ch == '.');
    match base {
        "sh" | "bash" | "zsh" => Some(FileType::Bash),
        "python" => Some(FileType::Python),
        "node" | "nodejs" | "bun" => Some(FileType::JavaScript),
        "ruby" => Some(FileType::Ruby),
        _ => None,
    }
}

/// Whole-word containment over ASCII identifier boundaries.
pub fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    text.match_indices(word).any(|(start, _)| {
        let end = start + word.len();
        (start == 0 || !is_ident(bytes[start - 1])) && (end == bytes.len() || !is_ident(bytes[end]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_for_every_file_type() {
        for file_type in ALL {
            assert_eq!(FileType::from_id(file_type.id()), Some(file_type));
            assert_eq!(FileType::from_u8(file_type as u8), Some(file_type));
        }
        assert_eq!(FileType::from_u8(0), None);
        assert_eq!(FileType::from_u8(73), None);
    }

    #[test]
    fn detect_uses_longest_suffix_and_special_names() {
        let cases = [
            ("main.go", Detection::Known(FileType::Go)),
            ("View.blade.php", Detection::Known(FileType::Blade)),
            ("index.php", Detection::Known(FileType::Php)),
            ("App.TSX", Detection::Known(FileType::TypeScript)),
            ("analysis.R", Detection::Known(FileType::R)),
            ("solver.F90", Detection::Known(FileType::Fortran)),
            ("Dockerfile", Detection::Known(FileType::Dockerfile)),
            ("api.dockerfile", Detection::Known(FileType::Dockerfile)),
            ("Makefile", Detection::Known(FileType::Make)),
            ("rules.mk", Detection::Known(FileType::Make)),
            (".env.example", Detection::Known(FileType::EnvExample)),
            ("lib.h", Detection::Ambiguous(Ambiguous::Header)),
            ("x.cls", Detection::Ambiguous(Ambiguous::Cls)),
            ("run", Detection::Shebang),
            (".gitignore", Detection::Unknown),
            ("archive.zip", Detection::Unknown),
        ];
        for (name, want) in cases {
            assert_eq!(detect(name), want, "{name}");
        }
    }

    #[test]
    fn ambiguous_suffixes_follow_content_markers() {
        use Ambiguous::*;
        let cases: [(Ambiguous, &str, bool, Option<FileType>); 12] = [
            (
                Header,
                "#import <Foundation/Foundation.h>",
                false,
                Some(FileType::ObjC),
            ),
            (Header, "namespace app { }", false, Some(FileType::Cpp)),
            (Header, "int add(int a);", true, Some(FileType::Cpp)),
            (Header, "int add(int a);", false, Some(FileType::C)),
            (Header, "int classify(int a);", false, Some(FileType::C)),
            (M, "@implementation Foo\n@end", false, Some(FileType::ObjC)),
            (M, "x = [1 2 3];", false, None),
            (V, "module top(); endmodule", false, Some(FileType::Verilog)),
            (
                PascalInclude,
                "procedure Run; begin end;",
                false,
                Some(FileType::Pascal),
            ),
            (Cl, "  (defun f ())", false, Some(FileType::Lisp)),
            (
                Cls,
                "public with sharing class Account {}",
                false,
                Some(FileType::Apex),
            ),
            (Cls, "\\documentclass{article}", false, None),
        ];
        for (ambiguous, head, sibling, want) in cases {
            assert_eq!(
                resolve_ambiguous(ambiguous, head.as_bytes(), sibling),
                want,
                "{head}"
            );
        }
    }

    #[test]
    fn shell_shebangs_are_recognized() {
        assert_eq!(shebang_type(b"#!/bin/sh\necho"), Some(FileType::Bash));
        assert_eq!(shebang_type(b"#!/usr/bin/env bash\n"), Some(FileType::Bash));
        assert_eq!(
            shebang_type(b"#!/usr/bin/env -S zsh -f\n"),
            Some(FileType::Bash)
        );
        assert_eq!(
            shebang_type(b"#!/usr/bin/env python3\n"),
            Some(FileType::Python)
        );
        assert_eq!(
            shebang_type(b"#!/usr/local/bin/python3.12\n"),
            Some(FileType::Python)
        );
        assert_eq!(shebang_type(b"#!/usr/bin/perl\n"), None);
        assert_eq!(shebang_type(b"echo hi"), None);
    }
}
