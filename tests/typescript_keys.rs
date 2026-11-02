//! End-to-end field-list encoding, persistence and budget checks.

mod support;

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache");
        fs::create_dir_all(&cache)?;
        if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
            return Err("test cache must not be a symlink".into());
        }
        for sequence in 0..1000 {
            let path = cache.join(format!("typescript-keys-{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err("no unused test directory".into())
    }

    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        Ok(support::command(env!("CARGO_BIN_EXE_xcalygraph"), &self.0)
            .args(arguments)
            .arg("-C")
            .arg(self.0.join("repo"))
            .arg("--index-dir")
            .arg(self.0.join("index"))
            .output()?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn typescript_type_names_respect_lexical_visibility() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    for (name, source) in [
        (
            "valid.ts",
            &include_bytes!("fixtures/typescript-lexical-valid.ts")[..],
        ),
        (
            "invalid.ts",
            &include_bytes!("fixtures/typescript-lexical-invalid.ts")[..],
        ),
        (
            "inherited.ts",
            &include_bytes!("fixtures/typescript-inherited-ambiguous.ts")[..],
        ),
        (
            "provider.ts",
            &include_bytes!("fixtures/typescript-lexical-provider.ts")[..],
        ),
        (
            "consumer.ts",
            &include_bytes!("fixtures/typescript-lexical-consumer.ts")[..],
        ),
        (
            "private-import.ts",
            &include_bytes!("fixtures/typescript-lexical-private-import.ts")[..],
        ),
        (
            "scope-augment.ts",
            &include_bytes!("fixtures/typescript-lexical-augmentation.ts")[..],
        ),
    ] {
        fs::write(root.join(name), source)?;
    }
    let long_names: Vec<_> = [140, 145, 150, 190]
        .into_iter()
        .enumerate()
        .map(|(index, length)| {
            (
                format!("long{index}"),
                format!("function{}", "x".repeat(length)),
            )
        })
        .collect();
    for (file, long_name) in &long_names {
        let source = format!(
            "export class Service {{ run(): number {{ return 1; }} }}\nexport function {long_name}<Service extends {{ run(): number }}>(value: Service): number {{ return value.run(); }}\n"
        );
        fs::write(root.join(format!("{file}.ts")), source)?;
    }
    for (caller, expected, wrong) in [
        (
            "valid.outside",
            "valid.Service.run",
            "valid.Area.Service.run",
        ),
        (
            "valid.Area.inside",
            "valid.Area.Service.run",
            "valid.Service.run",
        ),
        (
            "inherited.run",
            "inherited.Service.execute",
            "inherited.OtherService.execute",
        ),
        (
            "consumer.viaImport",
            "provider.Imported.run",
            "consumer.Other.Imported.run",
        ),
    ] {
        let result = fixture.run(&["show", caller, "--exact"])?;
        assert!(result.status.success());
        let text = String::from_utf8(result.stdout)?;
        assert!(
            text.contains(expected) && !text.contains(wrong),
            "{caller}: {text}"
        );
    }
    let result = fixture.run(&["show", "invalid.outside", "--exact", "--read-only"])?;
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout)?;
    assert!(!text.contains("invalid.Other.Hidden.run"), "{text}");
    let result = fixture.run(&[
        "show",
        "private-import.usePrivate",
        "--exact",
        "--read-only",
    ])?;
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout)?;
    assert!(!text.contains("provider.Private.run"), "{text}");
    let result = fixture.run(&[
        "show",
        "scope-augment.outsideAugmentation",
        "--exact",
        "--read-only",
    ])?;
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout)?;
    assert!(!text.contains("scope-augment.Imported.extra"), "{text}");
    for caller in [
        "valid.generic",
        "valid.genericArrow",
        "valid.arrow",
        "valid.asyncArrow",
        "valid.genericField",
    ] {
        let result = fixture.run(&["show", caller, "--exact", "--read-only"])?;
        assert!(result.status.success());
        let text = String::from_utf8(result.stdout)?;
        assert!(!text.contains("valid.Service.run"), "{text}");
    }
    for (file, name) in &long_names {
        let result = fixture.run(&["show", &format!("{file}.{name}"), "--exact", "--read-only"])?;
        assert!(result.status.success());
        let text = String::from_utf8(result.stdout)?;
        assert!(!text.contains(&format!("{file}.Service.run")), "{text}");
    }
    let impact = fixture.run(&["impact", "valid.Service.run", "--exact", "--read-only"])?;
    assert!(impact.status.success());
    let text = String::from_utf8(impact.stdout)?;
    assert!(
        text.contains("valid.outside") && !text.contains("valid.Area.inside"),
        "{text}"
    );
    assert_eq!(
        fixture
            .run(&["show", "valid.outside", "-p", "invalid.ts", "--read-only"])?
            .status
            .code(),
        Some(4)
    );
    Ok(())
}

#[test]
fn typescript_index_signatures_merge_only_with_proven_member_values() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    for (name, source) in [
        (
            "index-valid.ts",
            &include_bytes!("fixtures/typescript-index-signature-valid.ts")[..],
        ),
        (
            "index-invalid.ts",
            &include_bytes!("fixtures/typescript-index-signature-invalid.ts")[..],
        ),
    ] {
        fs::write(root.join(name), source)?;
    }

    let valid = fixture.run(&["show", "index-valid.useBag", "--exact"])?;
    assert!(valid.status.success());
    let valid = String::from_utf8(valid.stdout)?;
    assert!(valid.contains("index-valid.Service.run"), "{valid}");
    let impact = fixture.run(&[
        "impact",
        "index-valid.Service.run",
        "--exact",
        "--read-only",
    ])?;
    assert!(impact.status.success());
    let impact = String::from_utf8(impact.stdout)?;
    assert!(impact.contains("index-valid.useBag"), "{impact}");
    let filtered = fixture.run(&[
        "show",
        "index-valid.useBag",
        "-p",
        "index-valid.ts",
        "--read-only",
    ])?;
    assert!(filtered.status.success());
    assert!(String::from_utf8(filtered.stdout)?.contains("index-valid.Service.run"));
    assert_eq!(
        fixture
            .run(&[
                "show",
                "index-valid.useBag",
                "-p",
                "index-invalid.ts",
                "--read-only",
            ])?
            .status
            .code(),
        Some(4)
    );

    for (caller, invalid_target) in [
        ("index-invalid.useBad", "index-invalid.Service.run"),
        ("index-invalid.useDuplicate", "index-invalid.Service.run"),
        ("index-invalid.useMismatch", "index-invalid.Service.run"),
    ] {
        let result = fixture.run(&["show", caller, "--exact", "--read-only"])?;
        assert!(result.status.success());
        let output = String::from_utf8(result.stdout)?;
        assert!(!output.contains(invalid_target), "{caller}: {output}");
    }
    Ok(())
}

#[test]
fn typescript_field_lists_are_lossless_and_budgeted() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    fs::create_dir(fixture.0.join("repo"))?;
    fs::write(
        fixture.0.join("repo/keys.ts"),
        include_bytes!("fixtures/typescript-string-property-keys.ts"),
    )?;
    let tokens = r#""a,b","",escapeda,"line\u000afeed",😀,"\ud800","\\uD800""#;
    let text_line = format!("f {}", tokens.replace('\\', "\\\\"));
    let json_name = tokens.replace('\\', "\\\\").replace('"', "\\\"");
    let json_line = format!("\"rel\":\"fields\",\"name\":\"{json_name}\"");
    for (name, json, expected) in [
        ("SpecialKeys", false, text_line.as_str()),
        ("SpecialKeys", true, json_line.as_str()),
        ("SeparateKeys", false, "f a,b"),
        ("SeparateKeys", true, "\"rel\":\"fields\",\"name\":\"a,b\""),
        ("ComputedKey", false, "f regular"),
    ] {
        let mut args = vec!["show", name];
        if json {
            args.push("--json");
        }
        let output = fixture.run(&args)?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{args:?}: {text}");
        assert!(text.contains(expected), "{args:?}: {text}");
        args.push("--read-only");
        let repeated = fixture.run(&args)?;
        assert!(repeated.status.success());
        assert_eq!(repeated.stdout, text.as_bytes());
    }
    let typed = fixture.run(&["show", "useTyped", "--read-only"])?;
    assert!(typed.status.success());
    let typed = String::from_utf8(typed.stdout)?;
    assert!(typed.contains("> keys.Service.run "), "{typed}");
    assert!(!typed.contains("OtherService.run"), "{typed}");
    for json in [false, true] {
        for budget in [10, 20, 40, 80, 160] {
            let budget_text = budget.to_string();
            let mut args = vec!["show", "SpecialKeys", "--read-only", "-b", &budget_text];
            if json {
                args.push("--json");
            }
            let output = fixture.run(&args)?;
            assert!(output.status.success());
            assert!(output.stdout.len() <= budget * 3);
            let text = String::from_utf8(output.stdout)?;
            if json && text.contains("\"rel\":\"fields\"") {
                assert!(text.contains(&json_line), "{text}");
            }
            if !json && text.lines().any(|line| line.starts_with("f ")) {
                assert!(text.lines().any(|line| line == text_line), "{text}");
            }
        }
    }
    Ok(())
}

#[test]
fn typescript_numeric_keys_are_canonical_in_the_persisted_card() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    fs::create_dir(fixture.0.join("repo"))?;
    fs::write(
        fixture.0.join("repo/keys.ts"),
        include_bytes!("fixtures/typescript-numeric-property-keys.ts"),
    )?;
    let names = "16,1000,10,63,1234,92862869855095860,679496604817411.2,1e-7,0.000001,1e+21,100000000000000000000,Infinity,9007199254740992";
    let text = fixture.run(&["show", "NumericKeys"])?;
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout)?;
    assert!(
        text.lines().any(|line| line == format!("f {names}")),
        "{text}"
    );
    let json = fixture.run(&["show", "NumericKeys", "--json", "--read-only"])?;
    assert!(json.status.success());
    let json = String::from_utf8(json.stdout)?;
    assert!(
        json.contains(&format!("\"rel\":\"fields\",\"name\":\"{names}\"")),
        "{json}"
    );
    let ambiguous = fixture.run(&["show", "AmbiguousDecimalKey", "--read-only"])?;
    assert!(ambiguous.status.success());
    let ambiguous = String::from_utf8(ambiguous.stdout)?;
    assert!(
        ambiguous.lines().any(|line| line == "f safe"),
        "{ambiguous}"
    );
    Ok(())
}

#[test]
fn equivalent_interface_property_spellings_merge_only_when_compatible() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-equivalent-valid.ts"),
    )?;
    fs::write(
        root.join("invalid.ts"),
        include_bytes!("fixtures/typescript-equivalent-invalid.ts"),
    )?;
    for (name, lines) in [
        ("Strings", [1, 2]),
        ("Escaped", [3, 4]),
        ("Numbers", [5, 6]),
    ] {
        let output = fixture.run(&["show", &format!("valid.{name}")])?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{name}: {text}");
        for line in lines {
            assert!(text.contains(&format!("valid.ts:{line}")), "{name}: {text}");
        }
    }
    for name in [
        "DifferentTypes",
        "DifferentOptional",
        "DifferentReadonly",
        "DifferentLiteral",
    ] {
        let output = fixture.run(&["show", &format!("invalid.{name}")])?;
        assert_eq!(output.status.code(), Some(3), "{name}");
    }
    let impact = fixture.run(&["impact", "valid.ts:2", "--read-only", "--exact"])?;
    assert!(impact.status.success());
    let impact = String::from_utf8(impact.stdout)?;
    assert!(impact.contains("valid.readValue"), "{impact}");
    let filtered = fixture.run(&["show", "Strings", "--read-only", "-p", "invalid.ts"])?;
    assert_eq!(filtered.status.code(), Some(4));
    let distinct = fixture.run(&["show", "valid.EscapedIdentifier", "--read-only"])?;
    assert!(distinct.status.success());
    let distinct = String::from_utf8(distinct.stdout)?;
    assert!(
        distinct.contains("valid.ts:8") && distinct.contains("valid.ts:9"),
        "{distinct}"
    );
    assert!(distinct.lines().any(|line| line == "f a"), "{distinct}");
    assert!(
        distinct
            .lines()
            .any(|line| line.starts_with("f ") && line.contains("u0061")),
        "{distinct}"
    );
    Ok(())
}

#[test]
fn escaped_identifier_keys_normalize_across_fields_members_and_references()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    for (name, source) in [
        (
            "escaped.ts",
            &include_bytes!("fixtures/typescript-escaped-identifiers-valid.ts")[..],
        ),
        (
            "conflict.ts",
            &include_bytes!("fixtures/typescript-escaped-identifiers-conflict.ts")[..],
        ),
    ] {
        fs::write(root.join(name), source)?;
    }
    let indexed = fixture.run(&["index"])?;
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );

    let escaped = fixture.run(&["show", "escaped.EscapedField", "--exact", "--read-only"])?;
    assert!(
        escaped.status.success(),
        "{}",
        String::from_utf8_lossy(&escaped.stderr)
    );
    let escaped = String::from_utf8(escaped.stdout)?;
    assert!(
        escaped.contains("escaped.ts:7") && escaped.contains("escaped.ts:11"),
        "{escaped}"
    );
    assert!(escaped.lines().any(|line| line == "f a"), "{escaped}");

    for (caller, target) in [
        ("escaped.useEscapedField", "escaped.Service.run"),
        ("escaped.useUnicodeField", "escaped.Service.run"),
        ("escaped.useClassField", "escaped.Service.run"),
    ] {
        let output = fixture.run(&["show", caller, "--exact", "--read-only"])?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{caller}: {text}");
        assert!(text.contains(target), "{caller}: {text}");
        assert!(!text.contains("ambiguous references"), "{caller}: {text}");
        let repeated = fixture.run(&["show", caller, "--exact", "--read-only"])?;
        assert_eq!(repeated.stdout, text.as_bytes(), "{caller}");
    }
    let method = fixture.run(&["show", "escaped.useEscapedMethod", "--read-only"])?;
    let method_text = String::from_utf8(method.stdout)?;
    assert!(method.status.success(), "{method_text}");
    assert!(
        method_text.contains("escaped.EscapedMethod.do"),
        "{method_text}"
    );
    let repeated = fixture.run(&["show", "escaped.useEscapedMethod", "--read-only"])?;
    assert_eq!(repeated.stdout, method_text.as_bytes());

    let unicode = fixture.run(&["show", "escaped.UnicodeField", "--exact", "--read-only"])?;
    assert!(
        unicode.status.success(),
        "{}",
        String::from_utf8_lossy(&unicode.stderr)
    );
    let unicode = String::from_utf8(unicode.stdout)?;
    assert!(unicode.lines().any(|line| line == "f α"), "{unicode}");
    let distinct = fixture.run(&["show", "escaped.DistinctKeys", "--exact", "--read-only"])?;
    assert!(
        distinct.status.success(),
        "{}",
        String::from_utf8_lossy(&distinct.stderr)
    );
    let distinct = String::from_utf8(distinct.stdout)?;
    assert!(
        distinct.contains("f a,") && distinct.contains("u0061"),
        "{distinct}"
    );

    let malformed_fixture = Fixture::new()?;
    let malformed_root = malformed_fixture.0.join("repo");
    fs::create_dir(&malformed_root)?;
    fs::write(
        malformed_root.join("malformed.ts"),
        include_bytes!("fixtures/typescript-escaped-identifiers-invalid.ts"),
    )?;
    let indexed = malformed_fixture.run(&["index"])?;
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    for name in ["MalformedClass", "MalformedInterface"] {
        let output = malformed_fixture.run(&[
            "show",
            &format!("malformed.{name}"),
            "--json",
            "--read-only",
        ])?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{name}: {text}");
        assert!(!text.contains("\"rel\":\"fields\""), "{name}: {text}");
    }
    assert_eq!(
        fixture
            .run(&["show", "conflict.Conflict", "--exact", "--read-only"])?
            .status
            .code(),
        Some(3)
    );

    let filtered = fixture.run(&[
        "show",
        "escaped.useEscapedField",
        "--exact",
        "--read-only",
        "-p",
        "escaped.ts",
    ])?;
    assert!(filtered.status.success());
    assert!(String::from_utf8(filtered.stdout)?.contains("escaped.Service.run"));
    assert_eq!(
        fixture
            .run(&[
                "show",
                "escaped.useEscapedField",
                "--read-only",
                "-p",
                "malformed.ts",
            ])?
            .status
            .code(),
        Some(4)
    );

    let impact = fixture.run(&["impact", "escaped.Service.run", "--exact", "--read-only"])?;
    assert!(
        impact.status.success(),
        "{}",
        String::from_utf8_lossy(&impact.stderr)
    );
    let impact = String::from_utf8(impact.stdout)?;
    for caller in ["useEscapedField", "useUnicodeField", "useClassField"] {
        assert!(impact.contains(caller), "{impact}");
    }
    let method_impact =
        fixture.run(&["impact", "escaped.EscapedMethod", "--exact", "--read-only"])?;
    assert!(
        method_impact.status.success(),
        "{}",
        String::from_utf8_lossy(&method_impact.stderr)
    );
    let method_impact = String::from_utf8(method_impact.stdout)?;
    assert!(
        method_impact.contains("useEscapedMethod"),
        "{method_impact}"
    );
    Ok(())
}

#[test]
fn inherited_interfaces_merge_without_hiding_incompatible_base_members()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-inherited-valid.ts"),
    )?;
    fs::write(
        root.join("invalid.ts"),
        include_bytes!("fixtures/typescript-inherited-invalid.ts"),
    )?;
    fs::write(
        root.join("script.ts"),
        include_bytes!("fixtures/typescript-inherited-script.ts"),
    )?;
    fs::write(
        root.join("alias.ts"),
        include_bytes!("fixtures/typescript-inherited-export-alias.ts"),
    )?;
    fs::write(
        root.join("augment.ts"),
        include_bytes!("fixtures/typescript-inherited-augment.ts"),
    )?;
    fs::write(
        root.join("ambiguous.ts"),
        include_bytes!("fixtures/typescript-inherited-ambiguous.ts"),
    )?;
    let show = fixture.run(&["show", "valid.Inherited"])?;
    let text = String::from_utf8(show.stdout)?;
    assert!(show.status.success(), "{text}");
    for expected in ["valid.ts:2", "valid.ts:3", "left", "right", "valid.Base"] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    let invalid = fixture.run(&["show", "invalid.Broken", "--read-only"])?;
    assert_eq!(invalid.status.code(), Some(3));
    for name in [
        "invalid.Unknown",
        "invalid.BrokenMethod",
        "valid.GenericChild",
        "script.GlobalChild",
        "alias.AliasChild",
        "invalid.Augmented",
        "invalid.Exposed",
    ] {
        assert_eq!(
            fixture.run(&["show", name, "--read-only"])?.status.code(),
            Some(3),
            "{name}"
        );
    }
    let caller = fixture.run(&["show", "valid.callBase", "--read-only", "--exact"])?;
    assert!(caller.status.success());
    let caller = String::from_utf8(caller.stdout)?;
    assert!(caller.contains("> valid.Base.run "), "{caller}");
    let caller = fixture.run(&["show", "valid.callField", "--read-only", "--exact"])?;
    assert!(caller.status.success());
    let caller = String::from_utf8(caller.stdout)?;
    assert!(caller.contains("> valid.Service.execute "), "{caller}");
    assert!(!caller.contains("OtherService.execute"), "{caller}");
    let base = fixture.run(&["show", "valid.Base", "--read-only"])?;
    assert!(base.status.success());
    let base = String::from_utf8(base.stdout)?;
    assert_eq!(base.matches("v valid.Inherited ").count(), 1, "{base}");
    let second = fixture.run(&["show", "valid.ts:3", "--read-only", "--exact"])?;
    assert!(second.status.success());
    let second = String::from_utf8(second.stdout)?;
    assert!(second.contains("^ valid.Base "), "{second}");
    let impact = fixture.run(&["impact", "valid.ts:3", "--read-only", "--exact"])?;
    assert!(impact.status.success());
    let impact = String::from_utf8(impact.stdout)?;
    assert!(
        impact.contains("valid.inspect") && impact.contains("valid.callBase"),
        "{impact}"
    );
    let ambiguous = fixture.run(&["show", "ambiguous.run", "--read-only", "--exact"])?;
    assert!(ambiguous.status.success());
    let ambiguous = String::from_utf8(ambiguous.stdout)?;
    assert!(
        ambiguous.contains("ambiguous.Service.execute"),
        "{ambiguous}"
    );
    assert!(
        !ambiguous.contains("ambiguous.OtherService.execute"),
        "{ambiguous}"
    );
    Ok(())
}

#[test]
fn imported_interface_bases_require_cross_file_compatibility() -> Result<(), Box<dyn Error>> {
    for (base, child, augment, valid) in [
        (
            &include_bytes!("fixtures/typescript-cross-base-valid.ts")[..],
            &include_bytes!("fixtures/typescript-cross-child-valid.ts")[..],
            None,
            true,
        ),
        (
            &include_bytes!("fixtures/typescript-cross-base-invalid.ts")[..],
            &include_bytes!("fixtures/typescript-cross-child-invalid.ts")[..],
            Some(&include_bytes!("fixtures/typescript-cross-augmentation.ts")[..]),
            false,
        ),
        (
            &include_bytes!("fixtures/typescript-cross-base-valid.ts")[..],
            &include_bytes!("fixtures/typescript-cross-child-conflict.ts")[..],
            None,
            false,
        ),
    ] {
        let fixture = Fixture::new()?;
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        fs::write(root.join("base.ts"), base)?;
        fs::write(root.join("child.ts"), child)?;
        if let Some(augment) = augment {
            fs::write(root.join("augment.ts"), augment)?;
        }
        let output = fixture.run(&["show", "child.Child"])?;
        if valid {
            let text = String::from_utf8(output.stdout)?;
            assert!(output.status.success(), "{text}");
            for expected in ["child.ts:2", "child.ts:3", "base.Base"] {
                assert!(text.contains(expected), "missing {expected}: {text}");
            }
            let public = fixture.run(&["show", "child.Public", "--read-only"])?;
            assert!(
                public.status.success(),
                "{}",
                String::from_utf8_lossy(&public.stdout)
            );
            let call = fixture.run(&["show", "child.call", "--exact", "--read-only"])?;
            assert!(call.status.success());
            let text = String::from_utf8(call.stdout)?;
            assert!(text.contains("base.Base.run"), "{text}");
            let impact = fixture.run(&["impact", "base.Base", "--exact", "--read-only"])?;
            assert!(impact.status.success());
            let text = String::from_utf8(impact.stdout)?;
            assert!(text.contains("child.call"), "{text}");
            let sibling = root.join("child.d.ts");
            fs::write(&sibling, "export interface Child { extra: string; }")?;
            let changed = fixture.run(&["show", "child.ts:2"])?;
            assert!(changed.status.success());
            let text = String::from_utf8(changed.stdout)?;
            assert!(!text.contains("child.ts:3"), "{text}");
            fs::remove_file(sibling)?;
            let incomplete = root.join("incomplete.ts");
            fs::write(&incomplete, "export interface Broken { item:")?;
            let changed = fixture.run(&["show", "child.Child"])?;
            assert_eq!(
                changed.status.code(),
                Some(3),
                "{}",
                String::from_utf8_lossy(&changed.stdout)
            );
            fs::remove_file(incomplete)?;
            let restored = fixture.run(&["show", "child.Child"])?;
            assert!(restored.status.success());
            let component = root.join("component.vue");
            fs::write(&component, "<script lang=\"ts\"></script>")?;
            let changed = fixture.run(&["show", "child.Child"])?;
            assert_eq!(
                changed.status.code(),
                Some(3),
                "{}",
                String::from_utf8_lossy(&changed.stdout)
            );
            fs::remove_file(component)?;
            fs::write(
                root.join("augment.ts"),
                include_bytes!("fixtures/typescript-cross-augmentation-after.ts"),
            )?;
            let changed = fixture.run(&["show", "child.Child"])?;
            assert_eq!(
                changed.status.code(),
                Some(3),
                "{}",
                String::from_utf8_lossy(&changed.stdout)
            );
        } else {
            assert_eq!(output.status.code(), Some(3));
        }
    }
    Ok(())
}

#[test]
fn augmented_imported_base_preserves_members_and_invalidates_compatibility()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("base.ts"),
        include_bytes!("fixtures/typescript-cross-base-valid.ts"),
    )?;
    fs::write(
        root.join("child.ts"),
        include_bytes!("fixtures/typescript-cross-child-augmented.ts"),
    )?;
    fs::write(
        root.join("augment.ts"),
        include_bytes!("fixtures/typescript-cross-augmentation-benign.ts"),
    )?;
    let child = fixture.run(&["show", "child.Child"])?;
    let text = String::from_utf8(child.stdout)?;
    assert!(child.status.success(), "{text}");
    assert!(
        text.contains("child.ts:2") && text.contains("child.ts:3"),
        "{text}"
    );
    let call = fixture.run(&["show", "child.callExtra", "--exact", "--read-only"])?;
    assert!(call.status.success());
    let text = String::from_utf8(call.stdout)?;
    assert!(
        text.contains("augment.Base.extra") && text.contains("augment.ts:2"),
        "{text}"
    );
    let impact = fixture.run(&["impact", "augment.Base.extra", "--exact", "--read-only"])?;
    assert!(impact.status.success());
    let text = String::from_utf8(impact.stdout)?;
    assert!(text.contains("child.callExtra"), "{text}");
    let base = fixture.run(&["show", "base.Base", "--read-only"])?;
    assert!(base.status.success());
    let text = String::from_utf8(base.stdout)?;
    assert!(
        text.contains("extra") && text.contains("augment.ts:2"),
        "{text}"
    );
    assert_eq!(text.matches("m extra(): number").count(), 1, "{text}");
    let entries = fs::read_dir(fixture.0.join("index"))?.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(entries.len(), 1);
    let derived = entries[0].path().join("derived");
    let mut image = fs::read(&derived)?;
    let section = 64 + 20 * 16;
    let offset = u64::from_le_bytes(image[section..section + 8].try_into()?) as usize;
    let length = u64::from_le_bytes(image[section + 8..section + 16].try_into()?) as usize;
    assert!(length >= 8 && length.is_multiple_of(8) && offset + length <= image.len());
    let entry = (offset..offset + length)
        .step_by(8)
        .find(|&entry| {
            let part = u32::from_le_bytes(image[entry..entry + 4].try_into().unwrap_or_default());
            let canonical =
                u32::from_le_bytes(image[entry + 4..entry + 8].try_into().unwrap_or_default());
            part < canonical
        })
        .ok_or("missing cross-file augmentation group")?;
    let part = image[entry..entry + 4].to_vec();
    image[entry + 4..entry + 8].copy_from_slice(&part);
    let checksum = xxhash_rust::xxh3::xxh3_64(&image[64..]);
    image[32..40].copy_from_slice(&checksum.to_le_bytes());
    fs::write(&derived, image)?;
    assert!(
        !fixture
            .run(&["show", "base.Base", "--read-only"])?
            .status
            .success()
    );
    assert!(fixture.run(&["show", "base.Base"])?.status.success());
    fs::write(
        root.join("augment.ts"),
        include_bytes!("fixtures/typescript-cross-augmentation-after.ts"),
    )?;
    assert_eq!(
        fixture.run(&["show", "child.Child"])?.status.code(),
        Some(3)
    );
    fs::write(
        root.join("augment.ts"),
        include_bytes!("fixtures/typescript-cross-augmentation-benign.ts"),
    )?;
    assert!(fixture.run(&["show", "child.Child"])?.status.success());
    fs::write(
        root.join("other.ts"),
        "export interface Other { id: string }",
    )?;
    fs::write(
        root.join("otheraugment.ts"),
        "import './other'; declare module './other' { interface Other { value: number } }",
    )?;
    assert!(fixture.run(&["show", "child.Child"])?.status.success());
    let unknown = root.join("global.ts");
    fs::write(
        &unknown,
        "export {}; declare global { interface Window { x: number } }",
    )?;
    assert_eq!(
        fixture.run(&["show", "child.Child"])?.status.code(),
        Some(3)
    );
    fs::remove_file(unknown)?;
    fs::write(
        root.join("augment.ts"),
        "import './missing'; declare module './missing' { interface Base { extra(): number; } }",
    )?;
    assert_eq!(
        fixture.run(&["show", "child.Child"])?.status.code(),
        Some(3)
    );
    Ok(())
}

#[test]
fn augmented_field_types_use_the_declaration_file() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("base.ts"),
        "class Service { run(): number { return 1; } }\nexport interface Base { id: string; }\n",
    )?;
    fs::write(
        root.join("augment.ts"),
        "import './base';\nexport class Service { run(): number { return 2; } }\ndeclare module './base' { interface Base { extra: Service; } }\n",
    )?;
    fs::write(
        root.join("child.ts"),
        "import type { Base } from './base';\ninterface Child extends Base { left: string }\ninterface Child extends Base { right: number }\nexport function call(value: Child): number { return value.extra.run(); }\n",
    )?;
    let output = fixture.run(&["show", "child.call", "--exact"])?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("augment.Service.run"), "{text}");
    assert!(!text.contains("base.Service"), "{text}");
    fs::write(
        root.join("base.ts"),
        "export class Service { run(): number { return 1; } }\nexport interface Base { id: string; }\n",
    )?;
    let output = fixture.run(&["show", "child.call", "--exact"])?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("base.Service.run"), "{text}");
    assert!(!text.contains("augment.Service.run"), "{text}");
    fs::write(
        root.join("base.ts"),
        "class Service { run(): number { return 1; } } export { Service }; export interface Base { id: string; }",
    )?;
    let output = fixture.run(&["show", "child.call", "--exact"])?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(!text.contains("augment.Service.run"), "{text}");
    Ok(())
}

#[test]
fn generic_defaults_preserve_substitution_and_reject_conflicts() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-generic-default-valid.ts"),
    )?;
    fs::write(
        root.join("invalid.ts"),
        include_bytes!("fixtures/typescript-generic-default-invalid.ts"),
    )?;
    for name in [
        "valid.Child",
        "valid.Explicit",
        "valid.Defaulted",
        "valid.BareDefault",
    ] {
        let result = fixture.run(&["show", name])?;
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        let text = String::from_utf8(result.stdout)?;
        let base = if name == "valid.Child" {
            "valid.Pair"
        } else if name == "valid.BareDefault" {
            "valid.DefaultBox"
        } else {
            "valid.Concrete"
        };
        assert!(text.contains(base), "{name}: {text}");
    }
    for name in [
        "invalid.Conflict",
        "invalid.Missing",
        "invalid.Unproven",
        "invalid.BadDefault",
    ] {
        assert_eq!(
            fixture.run(&["show", name, "--read-only"])?.status.code(),
            Some(3),
            "{name}"
        );
    }
    for name in ["valid.call", "valid.callDefault", "valid.callBare"] {
        let result = fixture.run(&["show", name, "--read-only", "--exact"])?;
        assert!(result.status.success());
        let text = String::from_utf8(result.stdout)?;
        assert!(text.contains("valid.Service.execute"), "{name}: {text}");
    }
    let impact = fixture.run(&["impact", "valid.Pair", "--read-only", "--exact"])?;
    assert!(impact.status.success());
    let text = String::from_utf8(impact.stdout)?;
    assert!(text.contains("valid.call"), "{text}");
    Ok(())
}

#[test]
fn transitive_and_multiple_generic_bases_preserve_real_targets() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-generic-default-valid.ts"),
    )?;
    fs::write(
        root.join("invalid.ts"),
        include_bytes!("fixtures/typescript-generic-default-invalid.ts"),
    )?;
    for name in [
        "valid.Transitive",
        "valid.Multiple",
        "valid.Diamond",
        "valid.ConstraintChild",
    ] {
        let output = fixture.run(&["show", name])?;
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    for name in [
        "valid.callTransitive",
        "valid.callMultiple",
        "valid.callDiamond",
        "valid.callConstrained",
    ] {
        let output = fixture.run(&["show", name, "--read-only", "--exact"])?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        assert!(text.contains("valid.Service.execute"), "{name}: {text}");
    }
    for name in [
        "invalid.BadTransitive",
        "invalid.BadMultiple",
        "invalid.Cyclic",
        "invalid.BadConstrainedDefault",
        "invalid.BadDependentDefault",
        "invalid.BadTransitiveConstraint",
        "invalid.BadGenericOverride",
    ] {
        assert_eq!(
            fixture.run(&["show", name, "--read-only"])?.status.code(),
            Some(3),
            "{name}"
        );
    }
    assert_eq!(
        fixture
            .run(&["show", "valid.MethodChild", "--read-only"])?
            .status
            .code(),
        Some(3)
    );
    let impact = fixture.run(&["impact", "valid.Root", "--read-only", "--exact"])?;
    assert!(impact.status.success());
    let text = String::from_utf8(impact.stdout)?;
    assert!(text.contains("valid.callTransitive"), "{text}");
    Ok(())
}

#[test]
fn generic_bases_have_source_backed_inheritance_edges() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-inherited-generic-valid.ts"),
    )?;
    let base = fixture.run(&["show", "valid.Box", "--exact"])?;
    assert!(base.status.success());
    let text = String::from_utf8(base.stdout)?;
    assert!(text.contains("v valid.Child"), "{text}");
    let impact = fixture.run(&["impact", "valid.Box", "--exact", "--read-only"])?;
    assert!(impact.status.success());
    let text = String::from_utf8(impact.stdout)?;
    assert!(text.contains("valid.Child"), "{text}");
    Ok(())
}

#[test]
fn generic_inherited_types_resolve_and_conflicts_remain_distinct() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-inherited-generic-valid.ts"),
    )?;
    fs::write(
        root.join("invalid.ts"),
        include_bytes!("fixtures/typescript-inherited-generic-invalid.ts"),
    )?;
    for name in [
        "valid.Child",
        "valid.Override",
        "valid.PairChild",
        "valid.Text",
    ] {
        let result = fixture.run(&["show", name])?;
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        let text = String::from_utf8(result.stdout)?;
        let expected = if name.ends_with("PairChild") {
            "valid.Pair"
        } else {
            "valid.Box"
        };
        assert!(text.contains(expected), "{name}: {text}");
    }
    for name in [
        "invalid.Conflict",
        "invalid.First",
        "invalid.BadConstraint",
        "invalid.BadQualified",
        "invalid.EscapedChild",
        "invalid.NonGenericChild",
        "invalid.Named",
    ] {
        assert_eq!(
            fixture.run(&["show", name, "--read-only"])?.status.code(),
            Some(3),
            "{name}"
        );
    }
    let call = fixture.run(&["show", "valid.call", "--read-only", "--exact"])?;
    assert!(call.status.success());
    let text = String::from_utf8(call.stdout)?;
    assert!(text.contains("valid.Service.execute"), "{text}");
    assert!(!text.contains("valid.T.execute"), "{text}");
    let second = fixture.run(&["show", "valid.second", "--read-only", "--exact"])?;
    assert!(second.status.success());
    let text = String::from_utf8(second.stdout)?;
    assert!(text.contains("valid.Other.other"), "{text}");
    let upper = fixture.run(&["show", "valid.upper", "--read-only", "--exact"])?;
    assert!(upper.status.success());
    let text = String::from_utf8(upper.stdout)?;
    assert!(!text.contains("valid.T.toUpperCase"), "{text}");
    let impact = fixture.run(&["impact", "valid.Box", "--read-only", "--exact"])?;
    assert!(impact.status.success());
    let text = String::from_utf8(impact.stdout)?;
    assert!(text.contains("valid.call"), "{text}");
    for name in ["valid.Nested", "valid.NestedOverride"] {
        let output = fixture.run(&["show", name, "--read-only"])?;
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    Ok(())
}

#[test]
fn complex_local_inheritance_keeps_valid_members_and_rejects_conflicts()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-inherited-complex-valid.ts"),
    )?;
    fs::write(
        root.join("invalid.ts"),
        include_bytes!("fixtures/typescript-inherited-complex-invalid.ts"),
    )?;
    let show = fixture.run(&["show", "valid.Child"])?;
    let text = String::from_utf8(show.stdout)?;
    assert!(show.status.success(), "{text}");
    for expected in ["valid.ts:4", "valid.ts:5", "valid.Left", "valid.Right"] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    let overload = fixture.run(&["show", "valid.OverChild", "--read-only"])?;
    assert!(
        overload.status.success(),
        "{}",
        String::from_utf8_lossy(&overload.stdout)
    );
    let different = fixture.run(&["show", "valid.Different", "--read-only"])?;
    let text = String::from_utf8(different.stdout)?;
    assert!(different.status.success(), "{text}");
    assert!(
        text.contains("valid.Root") && text.contains("valid.Right"),
        "{text}"
    );
    for name in [
        "invalid.Conflict",
        "invalid.Cycle",
        "invalid.Incomplete",
        "invalid.DifferentConflict",
    ] {
        assert_eq!(
            fixture.run(&["show", name, "--read-only"])?.status.code(),
            Some(3),
            "{name}"
        );
    }
    for (caller, target) in [
        ("valid.callRoot", "valid.Root.root"),
        ("valid.callService", "valid.Service.execute"),
        ("valid.callDifferent", "valid.Root.root"),
    ] {
        let output = fixture.run(&["show", caller, "--exact", "--read-only"])?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        assert!(text.contains(target), "{caller}: {text}");
    }
    let impact = fixture.run(&["impact", "valid.Root", "--exact", "--read-only"])?;
    assert!(impact.status.success());
    let impact = String::from_utf8(impact.stdout)?;
    assert!(impact.contains("valid.Child"), "{impact}");
    Ok(())
}

#[test]
fn identical_inherited_property_overrides_merge_without_accepting_conflicts()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-inherited-overrides-valid.ts"),
    )?;
    fs::write(
        root.join("invalid.ts"),
        include_bytes!("fixtures/typescript-inherited-overrides-invalid.ts"),
    )?;
    let valid = fixture.run(&["show", "valid.Child"])?;
    let text = String::from_utf8(valid.stdout)?;
    assert!(valid.status.success(), "{text}");
    for expected in ["valid.ts:2", "valid.ts:3", "valid.Base", "right"] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert_eq!(
        fixture
            .run(&["show", "invalid.Bad", "--read-only"])?
            .status
            .code(),
        Some(3)
    );
    let method = fixture.run(&["show", "valid.MethodChild", "--read-only"])?;
    let text = String::from_utf8(method.stdout)?;
    assert!(method.status.success(), "{text}");
    assert!(
        text.contains("valid.ts:6") && text.contains("valid.ts:7"),
        "{text}"
    );
    Ok(())
}

#[test]
fn same_file_interfaces_merge_without_losing_declaration_locations() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    fs::create_dir(fixture.0.join("repo"))?;
    for (name, source) in [
        (
            "model.ts",
            &include_bytes!("fixtures/typescript-merge-model.ts")[..],
        ),
        (
            "other.ts",
            &include_bytes!("fixtures/typescript-merge-other.ts")[..],
        ),
        (
            "invalid.ts",
            &include_bytes!("fixtures/typescript-merge-invalid.ts")[..],
        ),
    ] {
        fs::write(fixture.0.join("repo").join(name), source)?;
    }
    for name in ["model.Config", "model.ts:1", "model.ts:2"] {
        let output = fixture.run(&["show", name])?;
        let text = String::from_utf8(output.stdout)?;
        assert!(output.status.success(), "{name}: {text}");
        for required in ["width", "height", "model.ts:1", "model.ts:2"] {
            assert!(text.contains(required), "{name} lacks {required}: {text}");
        }
        assert!(!text.contains("other.ts"), "{text}");
    }
    for name in ["invalid.Broken", "invalid.Different"] {
        let output = fixture.run(&["show", name])?;
        assert_eq!(output.status.code(), Some(3), "{name}");
    }
    let other = fixture.run(&["show", "Config", "-p", "other.ts"])?;
    assert!(other.status.success());
    let other = String::from_utf8(other.stdout)?;
    assert!(
        other.contains("other") && !other.contains("width"),
        "{other}"
    );
    for caller in ["useService", "useImported"] {
        let typed = fixture.run(&["show", caller, "--read-only"])?;
        assert!(typed.status.success());
        let typed = String::from_utf8(typed.stdout)?;
        assert!(typed.contains("> model.Service.run "), "{caller}: {typed}");
        assert!(!typed.contains("OtherService.run"), "{caller}: {typed}");
    }
    let second_part = fixture.run(&["show", "model.ts:8", "--read-only"])?;
    assert!(second_part.status.success());
    let second_part = String::from_utf8(second_part.stdout)?;
    assert!(
        second_part
            .find("f extra")
            .zip(second_part.find("model.ts:7"))
            .is_some_and(|(fields, next)| fields < next),
        "fields must follow their own declaration: {second_part}"
    );
    for caller in ["useService", "useImported"] {
        assert!(
            second_part.contains(caller),
            "show second declaration lacks {caller}: {second_part}"
        );
    }
    let method = fixture.run(&["show", "useWorker", "--read-only"])?;
    assert!(method.status.success());
    let method = String::from_utf8(method.stdout)?;
    assert!(method.contains("> model.Worker.finish "), "{method}");
    let overload = fixture.run(&["show", "useQuery", "--exact", "--read-only"])?;
    assert!(overload.status.success());
    let overload = String::from_utf8(overload.stdout)?;
    assert!(
        overload.contains("> model.Query.execute model.ts:14"),
        "{overload}"
    );
    for name in ["model.Services", "model.ts:7", "model.ts:8"] {
        let impact = fixture.run(&["impact", name, "--read-only"])?;
        assert!(
            impact.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&impact.stderr)
        );
        let impact = String::from_utf8(impact.stdout)?;
        for caller in ["useService", "useImported"] {
            assert!(
                impact.contains(caller),
                "impact {name} lacks {caller}: {impact}"
            );
        }
    }
    let path = fixture.run(&["path", "useService", "model.ts:8", "--exact", "--read-only"])?;
    assert!(
        path.status.success(),
        "{}",
        String::from_utf8_lossy(&path.stderr)
    );
    let path = String::from_utf8(path.stdout)?;
    assert!(
        path.contains("useService") && path.contains("model.ts:8"),
        "{path}"
    );
    let found = fixture.run(&["find", "Services", "--json", "--read-only"])?;
    assert!(found.status.success());
    let found = String::from_utf8(found.stdout)?;
    assert_eq!(
        found.matches("\"name\":\"model.Services\"").count(),
        1,
        "{found}"
    );
    let context = fixture.run(&["ctx", "Services", "--json", "--read-only"])?;
    assert!(context.status.success());
    let context = String::from_utf8(context.stdout)?;
    assert_eq!(
        context.matches("\"name\":\"model.Services\"").count(),
        1,
        "{context}"
    );
    for command in ["find", "ctx"] {
        let exact = fixture.run(&[
            command,
            "exact symbol model.Services",
            "--json",
            "--read-only",
        ])?;
        assert!(
            exact.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&exact.stderr)
        );
        let exact = String::from_utf8(exact.stdout)?;
        assert_eq!(
            exact.matches("\"name\":\"model.Services\"").count(),
            1,
            "{command}: {exact}"
        );
    }
    let audit = fixture.run(&["audit", "--json", "--read-only"])?;
    assert!(audit.status.success());
    let audit = String::from_utf8(audit.stdout)?;
    for record in audit
        .lines()
        .filter(|line| line.contains("\"name\":\"model.Services\""))
    {
        assert!(!record.contains("\"rel\":\"unused\""), "{record}");
    }
    for (name, section) in [("model.Unused", "unused"), ("model.Private", "dead")] {
        let matches: Vec<_> = audit
            .lines()
            .filter(|line| {
                line.contains(&format!("\"name\":\"{name}\""))
                    && line.contains(&format!("\"rel\":\"{section}\""))
            })
            .collect();
        assert_eq!(matches.len(), 1, "{name}: {audit}");
    }
    let map = fixture.run(&[
        "map",
        "--json",
        "--read-only",
        "-p",
        "model.ts",
        "-p",
        "other.ts",
    ])?;
    assert!(map.status.success());
    let map = String::from_utf8(map.stdout)?;
    let hubs: Vec<_> = map
        .lines()
        .filter(|line| {
            line.contains("\"rel\":\"hub\"") && line.contains("\"name\":\"model.Services\"")
        })
        .collect();
    assert_eq!(hubs.len(), 1, "expected one logical hub: {map}");
    for name in ["Config", "Services", "Worker", "Query", "Unused", "Private"] {
        let matches = map
            .lines()
            .filter(|line| {
                line.contains("\"rel\":\"hub\"")
                    && line.contains(&format!("\"name\":\"model.{name}\""))
            })
            .count();
        assert!(matches <= 1, "duplicate {name}: {map}");
    }
    Ok(())
}

#[test]
fn reopened_namespace_interfaces_merge_only_within_matching_paths() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-namespace-merge-valid.ts"),
    )?;
    fs::write(
        root.join("invalid.ts"),
        include_bytes!("fixtures/typescript-namespace-merge-invalid.ts"),
    )?;

    let merged = fixture.run(&["show", "Api.Config", "--exact"])?;
    let merged_text = String::from_utf8(merged.stdout.clone())?;
    assert!(merged.status.success(), "{merged_text}");
    for expected in ["valid.ts:3", "valid.ts:8", "left", "right", "service"] {
        assert!(
            merged_text.contains(expected),
            "missing {expected}: {merged_text}"
        );
    }
    assert!(!merged_text.contains("other"), "{merged_text}");
    let filtered = fixture.run(&[
        "show",
        "Api.Config",
        "--exact",
        "--read-only",
        "-p",
        "valid.ts",
    ])?;
    assert!(filtered.status.success());
    assert_eq!(filtered.stdout, merged.stdout);
    let repeated = fixture.run(&["show", "Api.Config", "--exact", "--read-only"])?;
    assert_eq!(repeated.stdout, merged.stdout);
    for line in ["valid.ts:3", "valid.ts:8"] {
        let by_line = fixture.run(&["show", line, "--exact", "--read-only"])?;
        let text = String::from_utf8(by_line.stdout)?;
        assert!(by_line.status.success(), "{line}: {text}");
        for expected in ["valid.ts:3", "valid.ts:8", "left", "right"] {
            assert!(
                text.contains(expected),
                "{line}: missing {expected}: {text}"
            );
        }
    }

    let nested = fixture.run(&["show", "Root.Nested.Data", "--exact", "--read-only"])?;
    let nested_text = String::from_utf8(nested.stdout)?;
    assert!(nested.status.success(), "{nested_text}");
    for expected in ["valid.ts:16", "valid.ts:20", "left", "right"] {
        assert!(
            nested_text.contains(expected),
            "missing {expected}: {nested_text}"
        );
    }

    let separate = fixture.run(&["show", "Other.Config", "--exact", "--read-only"])?;
    let separate_text = String::from_utf8(separate.stdout)?;
    assert!(separate.status.success(), "{separate_text}");
    assert!(separate_text.contains("other"), "{separate_text}");
    assert!(!separate_text.contains("left"), "{separate_text}");
    assert!(!separate_text.contains("right"), "{separate_text}");
    assert_eq!(
        fixture
            .run(&["show", "Broken.Model", "--read-only"])?
            .status
            .code(),
        Some(3)
    );
    assert_eq!(
        fixture
            .run(&["show", "Config", "--read-only"])?
            .status
            .code(),
        Some(3)
    );

    let caller = fixture.run(&["show", "Api.useSecond", "--exact", "--read-only"])?;
    let caller_text = String::from_utf8(caller.stdout)?;
    assert!(caller.status.success(), "{caller_text}");
    assert!(
        caller_text.contains("valid.Api.Service.run"),
        "{caller_text}"
    );
    let impact = fixture.run(&["impact", "Api.Config", "--exact", "--read-only"])?;
    let impact_text = String::from_utf8(impact.stdout)?;
    assert!(impact.status.success(), "{impact_text}");
    for expected in ["useFirst", "useSecond"] {
        assert!(
            impact_text.contains(expected),
            "missing {expected}: {impact_text}"
        );
    }
    Ok(())
}

#[test]
fn invalid_interface_group_is_rejected_and_rebuilt() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    fs::create_dir(fixture.0.join("repo"))?;
    fs::write(
        fixture.0.join("repo/model.ts"),
        include_bytes!("fixtures/typescript-merge-model.ts"),
    )?;
    assert!(fixture.run(&["show", "model.Services"])?.status.success());
    let entries = fs::read_dir(fixture.0.join("index"))?.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(entries.len(), 1);
    let path = entries[0].path().join("derived");
    let mut image = fs::read(&path)?;
    let group_entry = 64 + 20 * 16;
    let offset = u64::from_le_bytes(image[group_entry..group_entry + 8].try_into()?) as usize;
    let length = u64::from_le_bytes(image[group_entry + 8..group_entry + 16].try_into()?) as usize;
    assert!(length >= 8 && offset + length <= image.len());
    image[offset + 4..offset + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    let checksum = xxhash_rust::xxh3::xxh3_64(&image[64..]);
    image[32..40].copy_from_slice(&checksum.to_le_bytes());
    fs::write(&path, image)?;
    assert!(
        !fixture
            .run(&["show", "model.Services", "--read-only"])?
            .status
            .success()
    );
    let recovered = fixture.run(&["show", "model.Services"])?;
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    let recovered = String::from_utf8(recovered.stdout)?;
    assert!(
        recovered.contains("first") && recovered.contains("extra"),
        "{recovered}"
    );
    Ok(())
}

#[test]
fn merged_interfaces_are_deterministic_across_worker_counts() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("model.ts"),
        include_bytes!("fixtures/typescript-merge-model.ts"),
    )?;
    fs::write(
        root.join("valid.ts"),
        include_bytes!("fixtures/typescript-equivalent-valid.ts"),
    )?;
    fs::write(
        root.join("inherited.ts"),
        include_bytes!("fixtures/typescript-inherited-valid.ts"),
    )?;
    fs::write(
        root.join("overrides.ts"),
        include_bytes!("fixtures/typescript-inherited-overrides-valid.ts"),
    )?;
    fs::write(
        root.join("complex.ts"),
        include_bytes!("fixtures/typescript-inherited-complex-valid.ts"),
    )?;
    fs::write(
        root.join("generic.ts"),
        include_bytes!("fixtures/typescript-inherited-generic-valid.ts"),
    )?;
    fs::write(
        root.join("defaults.ts"),
        include_bytes!("fixtures/typescript-generic-default-valid.ts"),
    )?;
    fs::write(
        root.join("lexical.ts"),
        include_bytes!("fixtures/typescript-lexical-valid.ts"),
    )?;
    fs::write(
        root.join("index-signature.ts"),
        include_bytes!("fixtures/typescript-index-signature-valid.ts"),
    )?;
    fs::write(
        root.join("escaped.ts"),
        include_bytes!("fixtures/typescript-escaped-identifiers-valid.ts"),
    )?;
    fs::write(
        root.join("base.ts"),
        include_bytes!("fixtures/typescript-cross-base-valid.ts"),
    )?;
    fs::write(
        root.join("child.ts"),
        include_bytes!("fixtures/typescript-cross-child-valid.ts"),
    )?;
    fs::write(
        root.join("augment.ts"),
        include_bytes!("fixtures/typescript-cross-augmentation-benign.ts"),
    )?;
    fs::write(
        root.join("augchild.ts"),
        include_bytes!("fixtures/typescript-cross-child-augmented.ts"),
    )?;
    let mut images = Vec::new();
    let mut answers = Vec::new();
    let mut escaped_answers = Vec::new();
    for workers in ["1", "4"] {
        let index = fixture.0.join(format!("index-{workers}"));
        let run = |args: &[&str]| -> Result<Output, Box<dyn Error>> {
            Ok(
                support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
                    .args(args)
                    .arg("-C")
                    .arg(&root)
                    .arg("--index-dir")
                    .arg(&index)
                    .env("RAYON_NUM_THREADS", workers)
                    .output()?,
            )
        };
        let output = run(&["index"])?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let entries = fs::read_dir(&index)?.collect::<Result<Vec<_>, _>>()?;
        assert_eq!(entries.len(), 1);
        images.push(fs::read(entries[0].path().join("derived"))?);
        let answer = run(&["show", "model.Services", "--json", "--read-only"])?;
        assert!(answer.status.success());
        let equivalent = run(&["show", "valid.Strings", "--json", "--read-only"])?;
        assert!(equivalent.status.success());
        let inherited = run(&["show", "inherited.Inherited", "--json", "--read-only"])?;
        assert!(inherited.status.success());
        let overrides = run(&["show", "overrides.Child", "--json", "--read-only"])?;
        assert!(overrides.status.success());
        let complex = run(&["show", "complex.Child", "--json", "--read-only"])?;
        assert!(complex.status.success());
        let generic = run(&["show", "generic.Child", "--json", "--read-only"])?;
        assert!(generic.status.success());
        let defaults = run(&["show", "defaults.Child", "--json", "--read-only"])?;
        assert!(defaults.status.success());
        let transitive = run(&["show", "defaults.Transitive", "--json", "--read-only"])?;
        assert!(transitive.status.success());
        let imported = run(&["show", "child.Child", "--json", "--read-only"])?;
        assert!(imported.status.success());
        let augmented = run(&["show", "augchild.Child", "--json", "--read-only"])?;
        assert!(augmented.status.success());
        let lexical = run(&[
            "show",
            "lexical.outside",
            "--json",
            "--exact",
            "--read-only",
        ])?;
        assert!(lexical.status.success());
        let indexed = run(&[
            "show",
            "index-signature.useBag",
            "--json",
            "--exact",
            "--read-only",
        ])?;
        assert!(indexed.status.success());
        let escaped = run(&["show", "escaped.EscapedField", "--json", "--read-only"])?;
        assert!(escaped.status.success());
        let escaped_caller = run(&[
            "show",
            "escaped.useEscapedField",
            "--json",
            "--exact",
            "--read-only",
        ])?;
        assert!(escaped_caller.status.success());
        escaped_answers.push((escaped.stdout, escaped_caller.stdout));
        answers.push((
            answer.stdout,
            equivalent.stdout,
            inherited.stdout,
            overrides.stdout,
            complex.stdout,
            generic.stdout,
            defaults.stdout,
            transitive.stdout,
            imported.stdout,
            augmented.stdout,
            lexical.stdout,
            indexed.stdout,
        ));
    }
    assert_eq!(images[0], images[1]);
    assert_eq!(answers[0], answers[1]);
    assert_eq!(escaped_answers[0], escaped_answers[1]);
    Ok(())
}

#[test]
fn interface_second_part_diff_reaches_all_dependents() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    let source = include_str!("fixtures/typescript-merge-model.ts");
    fs::write(root.join("model.ts"), source)?;
    fs::write(
        root.join("other.ts"),
        include_bytes!("fixtures/typescript-merge-other.ts"),
    )?;
    for args in [
        &["init", "-q"][..],
        &["add", "model.ts", "other.ts"],
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "baseline",
        ],
    ] {
        let output = support::command("git", &fixture.0)
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(fixture.run(&["index"])?.status.success());
    assert!(source.contains("extra: Service"));
    fs::write(
        root.join("model.ts"),
        source.replace("extra: Service", "extra: OtherService"),
    )?;
    let output = fixture.run(&["impact", "--diff"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout)?;
    for expected in ["model.Services", "useService", "useImported"] {
        assert!(output.contains(expected), "missing {expected}: {output}");
    }
    for unrelated in ["Config", "Worker"] {
        assert!(
            !output.contains(unrelated),
            "unrelated {unrelated}: {output}"
        );
    }
    Ok(())
}
