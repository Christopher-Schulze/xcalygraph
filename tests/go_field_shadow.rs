//! Go fields take precedence over promoted methods with the same name.

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
fn field_call_does_not_resolve_to_a_promoted_method() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture(support::exclusive_dir("go-field-shadow")?);
    let root = &fixture.0;
    fs::write(
        root.join("main.go"),
        "package example\ntype Base struct{}\nfunc (Base) Run() {}\ntype Shadow struct { Base; Run func() }\ntype Multi struct { Base; Run, Other func() }\ntype Promoted struct { Base }\nfunc ShadowCall(x Shadow) { x.Run() }\nfunc MultiCall(x Multi) { x.Run() }\nfunc PromotedCall(x Promoted) { x.Run() }\n",
    )?;
    let mut outputs = Vec::new();
    for caller in ["ShadowCall", "MultiCall", "PromotedCall"] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), root)
            .args(["show", caller, "--json", "--all", "-C"])
            .arg(root)
            .output()?;
        assert!(output.status.success());
        outputs.push(String::from_utf8(output.stdout)?);
    }
    assert!(!outputs[0].contains("Base.Run"), "{}", outputs[0]);
    assert!(!outputs[1].contains("Base.Run"), "{}", outputs[1]);
    assert!(outputs[2].contains("Base.Run"), "{}", outputs[2]);
    Ok(())
}
