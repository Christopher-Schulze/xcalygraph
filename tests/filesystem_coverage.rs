//! Filesystem exploration must preserve source and configuration evidence.

mod support;

use std::error::Error;
use std::fs;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

#[test]
fn a_long_literal_does_not_hide_the_following_definition() -> Result<(), Box<dyn Error>> {
    let fixture = support::exclusive_dir("long-literal-source")?;
    let root = fixture.join("source");
    fs::create_dir(&root)?;
    fs::write(
        root.join("app.py"),
        format!(
            "DATA = \"{}\"\n\ndef long_literal_survivor():\n    return len(DATA)\n",
            "x".repeat(5000)
        ),
    )?;
    let output = support::command(BINARY, &fixture)
        .args(["show", "long_literal_survivor", "-C"])
        .arg(&root)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("app.py:3"), "{text}");
    assert!(text.contains("def long_literal_survivor"), "{text}");
    assert!(!text.contains("incomplete"), "{text}");
    fs::remove_dir_all(fixture)?;
    Ok(())
}

#[test]
fn server_configs_reference_literal_files_without_executing_mentions() -> Result<(), Box<dyn Error>>
{
    let fixture = support::exclusive_dir("server-config-paths")?;
    let root = fixture.join("source");
    fs::create_dir_all(root.join("config"))?;
    fs::write(
        root.join("config/server.conf"),
        concat!(
            "include child.conf;\n",
            "# include \"config/ignored.conf\";\n",
            "include ${CHOICE}.conf;\n",
            "include ../../outside.conf;\n",
        ),
    )?;
    for name in ["child", "ignored"] {
        fs::write(root.join(format!("config/{name}.conf")), "port 8080\n")?;
    }
    fs::write(root.join("outside.conf"), "port 9090\n")?;
    fs::write(
        root.join("config/app.service"),
        "[Service]\nEnvironmentFile=child.conf\n",
    )?;
    fs::write(root.join("config/app.yaml"), "include: child.conf\n")?;
    fs::write(
        root.join("config/app.json"),
        "{\n  // \"ignored.conf\"\n  \"include\": \"child.conf\"\n}\n",
    )?;
    fs::write(
        root.join("config/manifest.json"),
        "{\"/child.conf\": \"/child.conf?revision=1\"}\n",
    )?;
    fs::write(
        root.join("config/bom.json"),
        "\u{feff}[{\"include\": \"child.conf\"}]\n",
    )?;
    for source in [
        "config/server.conf",
        "config/app.service",
        "config/app.yaml",
        "config/app.json",
        "config/manifest.json",
        "config/bom.json",
    ] {
        let output = support::command(BINARY, &fixture)
            .args(["show", source, "-C"])
            .arg(&root)
            .output()?;
        assert!(output.status.success(), "{source}: {output:?}");
        let text = String::from_utf8(output.stdout)?;
        assert!(text.contains("config/child.conf"), "{source}: {text}");
        assert!(
            !text
                .lines()
                .any(|line| line.starts_with('>') && line.contains("config/ignored.conf")),
            "{source}: {text}"
        );
        assert!(!text.contains("outside.conf:1"), "{source}: {text}");
    }
    fs::remove_dir_all(fixture)?;
    Ok(())
}
