//! Callback values at module scope remain uses, without fabricated invocation edges.

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
fn module_callback_values_are_used_without_calling_or_unshadowing_them()
-> Result<(), Box<dyn Error>> {
    for (extension, source, site) in [
        (
            "js",
            "function _passed() { return 1; }\nfunction _shadowed() { return 2; }\nfunction _unused() { return 3; }\nfunction other(_shadowed) { register(_shadowed); }\nregister(_passed);\n",
            5,
        ),
        (
            "ts",
            "function _passed() { return 1; }\nfunction _shadowed() { return 2; }\nfunction _unused() { return 3; }\nfunction other(_shadowed) { register(_shadowed); }\nregister(_passed);\n",
            5,
        ),
        // Python parameters are not recorded as bindings (pytest injects fixtures through
        // them), so its case has no shadowing parameter.
        (
            "py",
            "def _passed(): return 1\ndef _shadowed(): return 2\ndef _unused(): return 3\nregister(_passed)\n",
            4,
        ),
    ] {
        let fixture = Fixture(support::exclusive_dir("callback-values")?);
        let root = fixture.0.join("repo");
        fs::create_dir(&root)?;
        let path = format!("callbacks.{extension}");
        fs::write(root.join(&path), source)?;
        for name in ["_passed", "_shadowed", "_unused"] {
            let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
                .args(["show", name, "--json", "--exact", "-C"])
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
            let incoming: Vec<_> = text
                .lines()
                .filter(|line| line.contains("\"rel\":\"in\""))
                .collect();
            if name == "_passed" {
                assert_eq!(incoming.len(), 1, "{extension}: {text}");
                assert!(
                    incoming[0].contains("\"kind\":\"uses\"")
                        && incoming[0].contains(&format!("\"path\":\"{path}\""))
                        && incoming[0].contains(&format!("\"line\":{site},")),
                    "{text}"
                );
            } else {
                assert!(incoming.is_empty(), "{extension} {name}: {text}");
            }
        }
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["audit", "--json", "--exact", "-C"])
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
        for (name, dead) in [("_passed", false), ("_shadowed", true), ("_unused", true)] {
            let found = text.lines().any(|line| {
                line.contains("\"rel\":\"dead\"")
                    && line.contains(&format!("\"name\":\"callbacks.{name}\""))
            });
            assert_eq!(found, dead, "{extension} {name}: {text}");
        }
    }
    Ok(())
}

#[test]
fn values_without_recorded_local_bindings_do_not_name_same_file_functions()
-> Result<(), Box<dyn Error>> {
    // Lua extraction records no local bindings, so `f` at line 2 may be the local of line 1
    // rather than the function of line 3: no `uses` edge is claimed.
    let fixture = Fixture(support::exclusive_dir("unrecorded-binding-values")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("values.lua"),
        "local f = load(\"return 1\")\nregister(f)\nfunction f() return 2 end\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "f", "--json", "-C"])
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
    assert!(
        !text
            .lines()
            .any(|line| line.contains("\"rel\":\"in\"") && line.contains("\"kind\":\"uses\"")),
        "{text}"
    );
    Ok(())
}

#[test]
fn pascal_callbacks_with_required_arguments_are_values() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("pascal-callback-values")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("callbacks.pas"),
        include_bytes!("fixtures/pascal-callback-values.pas"),
    )?;
    for (name, kind, site) in [("Handler", "uses", 12), ("Answer", "calls", 13)] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
            .args(["show", name, "--exact", "--json", "-C"])
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
        let incoming: Vec<_> = text
            .lines()
            .filter(|row| row.contains("\"rel\":\"in\""))
            .collect();
        assert_eq!(incoming.len(), 1, "{text}");
        assert!(
            incoming[0].contains(&format!("\"kind\":\"{kind}\""))
                && incoming[0].contains(&format!("\"line\":{site},")),
            "{text}"
        );
    }
    Ok(())
}

#[test]
fn python_default_values_are_uses_of_the_enclosing_binding() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("python-default-values")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("defaults.py"),
        "def _target(): return 1\ndef plain(callback=_target): return callback\ndef typed(callback: object = _target): return callback\n",
    )?;
    let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "_target", "--exact", "--json", "-C"])
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
    for (name, line) in [("defaults.plain", 2), ("defaults.typed", 3)] {
        assert!(
            text.lines().any(|row| row.contains("\"rel\":\"in\"")
                && row.contains(&format!("\"name\":\"{name}\""))
                && row.contains(&format!("\"line\":{line},"))
                && row.contains("\"kind\":\"uses\"")),
            "{text}"
        );
    }
    Ok(())
}
