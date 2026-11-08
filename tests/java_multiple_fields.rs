//! Each name in a Java field declaration retains its declared type.

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
fn both_java_fields_resolve_to_the_declared_type() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("java-multiple-fields")?);
    let root = &fixture.0;
    fs::write(
        root.join("Container.java"),
        "class Data { void execute() {} }\nclass Other { void execute() {} }\nclass Container {\n    Data first, second;\n    void firstCall() { first.execute(); }\n    void secondCall() { second.execute(); }\n}\n",
    )?;
    for caller in ["firstCall", "secondCall"] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), root)
            .args(["show", caller, "--json", "-C"])
            .arg(root)
            .output()?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        assert!(
            text.lines().any(|line| line.contains("Data.execute")
                && line.contains("\"kind\":\"calls\"")
                && line.contains("\"conf\":\"exact\"")),
            "{caller}: {text}"
        );
        assert!(!text.contains("Other.execute"), "{caller}: {text}");
    }
    Ok(())
}
