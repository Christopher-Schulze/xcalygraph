// ConfigYAML star line check for TASK 054 (local only, first slice)
use std::fs;

#[path = "support/mod.rs"]
mod support;

struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn config_yaml_keys_are_surfaced() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture(support::exclusive_dir("config-yaml-star")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("app.yml"),
        "server:\n  port: 8080\n  host: localhost\n",
    )?;
    let out = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "app.yml", "--json", "--all", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout)?;
    assert!(
        text.contains("ConfigYAML") || text.contains("server.port"),
        "expected config ghost, got: {text}"
    );
    fs::write(
        root.join("schema.json"),
        "{\n  \"name\": \"app\",\n  \"version\": \"1.0\"\n}\n",
    )?;
    let out2 = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "schema.json", "--json", "--all", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    assert!(
        out2.status.success(),
        "{}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let text2 = String::from_utf8(out2.stdout)?;
    assert!(
        text2.contains("SchemaJSON") || text2.contains("version"),
        "expected schema ghost, got: {text2}"
    );
    Ok(())
}
