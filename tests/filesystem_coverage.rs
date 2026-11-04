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
