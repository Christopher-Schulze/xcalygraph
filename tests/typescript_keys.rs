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
    let unknown = fixture.run(&["show", "valid.EscapedIdentifier", "--read-only"])?;
    assert_eq!(unknown.status.code(), Some(3));
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
    assert!(!ambiguous.contains("Service.execute"), "{ambiguous}");
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
    let mut images = Vec::new();
    let mut answers = Vec::new();
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
        answers.push((
            answer.stdout,
            equivalent.stdout,
            inherited.stdout,
            overrides.stdout,
            complex.stdout,
        ));
    }
    assert_eq!(images[0], images[1]);
    assert_eq!(answers[0], answers[1]);
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
