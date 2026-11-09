// Stable symbol IDs for TASK 228 segment experiment (local only, slice 1)
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
fn stable_id_survives_insert_before() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture(support::exclusive_dir("stable-id")?);
    let root = fixture.0.join("repo");
    fs::create_dir(&root)?;
    fs::write(root.join("b.rs"), "pub fn second() {}\n")?;
    let out = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "crate::second", "--json", "--all", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    let text = String::from_utf8(out.stdout)?;
    fs::write(root.join("a.rs"), "pub fn first() {}\n")?;
    let out2 = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &fixture.0)
        .args(["show", "crate::second", "--json", "--all", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(fixture.0.join("index"))
        .output()?;
    let text2 = String::from_utf8(out2.stdout)?;
    assert!(
        text.contains("crate::second") && text2.contains("crate::second"),
        "both contain\n{text}\n{text2}"
    );
    assert!(
        text.contains("stable_id") && text2.contains("stable_id"),
        "stable_id missing\n{text}\n{text2}"
    );
    fn extract(text: &str) -> String {
        for line in text.lines() {
            if line.contains("stable_id") {
                let Some(pos) = line.find("\"stable_id\":") else {
                    continue;
                };
                let start = pos + 12;
                let Some(rem) = line.get(start..) else {
                    continue;
                };
                let end = rem.find(',').or_else(|| rem.find('}')).unwrap_or(rem.len());
                let Some(slice) = line.get(start..start + end) else {
                    continue;
                };
                return slice.trim().trim_matches('"').to_owned();
            }
        }
        String::new()
    }
    let id1 = extract(&text);
    let id2 = extract(&text2);
    assert!(
        !id1.is_empty() && id1 == id2,
        "stable_id differs {id1} vs {id2}\n{text}\n{text2}"
    );
    // After implementation, text2 should contain "stable_id" field; until then this test passes as placeholder
    Ok(())
}
