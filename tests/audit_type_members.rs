//! An exported type is used when another file uses one of its members.

mod support;

use std::error::Error;
use std::fs;
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn exported_type_audit_follows_members_without_hiding_local_only_exports()
-> Result<(), Box<dyn Error>> {
    for extension in ["ts", "js"] {
        let fixture = Fixture(support::exclusive_dir("audit-type-members")?);
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        fs::write(
            root.join(format!("model.{extension}")),
            "export class Used { static create() { return 1; } }\n\
             export class LocalOnly { static create() { return 2; } }\n\
             export class RecursiveOnly { static create() { return RecursiveOnly.create(); } }\n\
             export class Unused { static create() { return 3; } }\n\
             export function localUse() { return LocalOnly.create(); }\n",
        )?;
        fs::write(
            root.join(format!("consumer.{extension}")),
            "import { Used } from './model.js';\n\
             export function request() { return Used.create(); }\n\
             class Other { static create() { return 4; } }\n\
             export function unrelated() { return Other.create(); }\n",
        )?;
        for flags in [vec![], vec!["--exact", "--no-tests", "-k", "class"]] {
            let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
                .args(["audit", "--json", "-p", &format!("model.{extension}")])
                .args(&flags)
                .arg("-C")
                .arg(&root)
                .arg("--index-dir")
                .arg(fixture.0.join("index"))
                .output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let text = String::from_utf8(output.stdout)?;
            for (name, unused) in [
                ("Used", false),
                ("LocalOnly", true),
                ("RecursiveOnly", true),
                ("Unused", true),
            ] {
                let found = text.lines().any(|line| {
                    line.contains("\"rel\":\"unused\"")
                        && line.contains(&format!("\"name\":\"model.{name}\""))
                });
                assert_eq!(found, unused, "{extension} {flags:?} {name}: {text}");
            }
        }
    }
    Ok(())
}
