//! Each name in a Go field declaration retains its declared type.

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
fn both_go_fields_resolve_to_the_declared_type() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("go-multiple-fields")?);
    let root = &fixture.0;
    fs::write(
        root.join("main.go"),
        "package example\ntype Data struct{}\nfunc (Data) Run() {}\ntype Other struct{}\nfunc (Other) Run() {}\ntype Container struct { First, Second Data }\nfunc FirstCall(c Container) { c.First.Run() }\nfunc SecondCall(c Container) { c.Second.Run() }\n",
    )?;
    for caller in ["FirstCall", "SecondCall"] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), root)
            .args(["show", caller, "--json", "--all", "-C"])
            .arg(root)
            .output()?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        assert!(
            text.lines()
                .any(|line| line.contains("Data.Run") && line.contains("\"kind\":\"calls\"")),
            "{caller}: {text}"
        );
        assert!(!text.contains("Other.Run"), "{caller}: {text}");
    }
    Ok(())
}
