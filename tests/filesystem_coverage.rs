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

#[test]
fn anchored_global_ignore_rules_do_not_depend_on_the_working_directory()
-> Result<(), Box<dyn Error>> {
    let base = support::exclusive_dir("global-ignore")?;
    let root = base.join("root");
    let elsewhere = base.join("elsewhere");
    fs::create_dir_all(root.join("sub"))?;
    fs::create_dir_all(&elsewhere)?;
    fs::write(
        root.join("sub/hidden.py"),
        "def hiddenglobal():\n    return 1\n",
    )?;
    fs::write(root.join("kept.py"), "def keptglobal():\n    return 1\n")?;
    fs::write(base.join("excludes"), "/sub/hidden.py\n")?;
    let config = base.join("gitconfig");
    fs::write(
        &config,
        format!(
            "[core]\n\texcludesFile = {}\n",
            base.join("excludes").display()
        ),
    )?;
    for (cwd, index) in [(&root, "inside"), (&elsewhere, "outside")] {
        let output = support::command(BINARY, &base)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", &config)
            .env("XCALYGRAPH_INDEX_DIR", base.join(index))
            .args(["map", "-C"])
            .arg(&root)
            .output()?;
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8(output.stdout)?;
        assert!(text.contains("1 files"), "{index}: {text}");
    }
    fs::remove_dir_all(base)?;
    Ok(())
}

fn bundle(functions: usize) -> String {
    let mut code = String::from("!function(e,t){");
    for index in 0..functions {
        code.push_str(&format!(
            "function f{index}(a,b){{return a+b+t}}var v{index}=f{index}(e,t);"
        ));
    }
    code.push_str("}(this,1);\n");
    code
}

#[test]
fn long_literals_stay_source_and_bundles_are_reported_as_not_parsed() -> Result<(), Box<dyn Error>>
{
    let root = support::exclusive_dir("minified-rule")?;
    let svg = "<svg><path d='M0 0;1 1'/>{x}(y);</svg>".repeat(160);
    let mut icons = String::new();
    for index in 0..17 {
        icons.push_str(&format!(
            "export function renderIcon{index}() {{ return \"{svg}\"; }}\n"
        ));
    }
    fs::write(root.join("icons.js"), icons)?;
    fs::write(
        root.join("table.js"),
        format!(
            "export function decodeTable(x) {{ return T[x]; }}\nconst T = [{}];\n",
            (0..3000)
                .map(|n| (n % 251).to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
    )?;
    fs::write(
        root.join("noted.js"),
        format!(
            "export function afterComment(x) {{ return x; }}\n// {}\n",
            "word; and { more } ( text ".repeat(300)
        ),
    )?;
    fs::write(root.join("bundle.js"), bundle(400))?;
    let run = |args: &[&str]| -> Result<(bool, String), Box<dyn Error>> {
        let output = support::command(BINARY, &root)
            .args(args)
            .arg("-C")
            .arg(&root)
            .output()?;
        let mut text = String::from_utf8(output.stdout)?;
        text.push_str(&String::from_utf8(output.stderr)?);
        Ok((output.status.success(), text))
    };
    for symbol in ["renderIcon16", "decodeTable", "afterComment"] {
        let (found, text) = run(&["show", symbol])?;
        assert!(found, "{symbol}: {text}");
    }
    let (found, text) = run(&["find", "f399"])?;
    assert!(!found, "{text}");
    assert!(text.contains("not parsed"), "{text}");
    assert!(text.contains("bundle.js"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}
