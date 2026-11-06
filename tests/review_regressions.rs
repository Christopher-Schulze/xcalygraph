//! False facts and false absences found by source review, one reproduction each.

mod support;

use std::error::Error;
use std::fs;
use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

type TestResult = Result<(), Box<dyn Error>>;

fn write(root: &Path, files: &[(&str, &[u8])]) -> TestResult {
    for (path, content) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(())
}

fn query(root: &Path, args: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = support::command(BINARY, root)
        .args(args)
        .arg("-C")
        .arg(root)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn an_export_alias_finds_its_declaration() -> TestResult {
    let root = support::exclusive_dir("export-alias")?;
    write(
        &root,
        &[(
            "a.ts",
            b"function hidden() { return 1; }\nexport { hidden as Public };\n",
        )],
    )?;
    let text = query(&root, &["show", "Public"])?;
    assert!(
        text.contains("fn hidden") || text.contains("hidden"),
        "{text}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

const SHADOW: &[u8] = b"<script>\nclass Outer { run() { return 1 } }\nclass Inner { run() { return 2 } }\nfunction check() { const value = new Outer(); { const value = new Inner(); value.run(); } value.run(); }\n</script>";

#[test]
fn html_scripts_keep_block_scopes() -> TestResult {
    let root = support::exclusive_dir("html-scope")?;
    write(&root, &[("page.html", SHADOW)])?;
    let text = query(&root, &["show", "check"])?;
    assert!(text.contains("Outer.run"), "{text}");
    assert!(text.contains("Inner.run"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_lossy_html_script_proves_no_receiver() -> TestResult {
    let root = support::exclusive_dir("html-lossy")?;
    let mut page = vec![0xff];
    page.extend_from_slice(SHADOW);
    write(&root, &[("page.html", &page)])?;
    let json = query(&root, &["show", "check", "--json"])?;
    assert!(
        !json
            .lines()
            .any(|line| line.contains("run") && line.contains("\"conf\":\"exact\"")),
        "{json}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
