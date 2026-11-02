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
