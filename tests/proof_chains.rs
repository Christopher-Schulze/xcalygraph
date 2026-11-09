//! `find` shows the edges that join its leading hits when no hit leads clearly.

mod support;

use std::error::Error;
use std::fs;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

fn fixture() -> Result<std::path::PathBuf, Box<dyn Error>> {
    let root = support::exclusive_dir("proof-chains")?;
    fs::write(
        root.join("entry.py"),
        "from worker import process_order\n\ndef handle_order(order):\n    return process_order(order)\n",
    )?;
    fs::write(
        root.join("worker.py"),
        "def process_order(order):\n    return store_order(order)\n\ndef store_order(order):\n    return order\n",
    )?;
    fs::write(
        root.join("other.py"),
        "def order_summary(order):\n    return order\n\ndef order_total(order):\n    return order\n",
    )?;
    Ok(root)
}

fn find(root: &std::path::Path, extra: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = support::command(BINARY, root)
        .args([
            "find", "order", "handle", "process", "store", "summary", "-C",
        ])
        .arg(root)
        .args(extra)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn leading_hits_show_their_source_located_connections() -> Result<(), Box<dyn Error>> {
    let root = fixture()?;
    let text = find(&root, &[])?;
    let chain = text
        .lines()
        .find(|line| line.starts_with("~ "))
        .ok_or_else(|| format!("no chain: {text}"))?;
    assert_eq!(
        chain, "~ handle_order entry.py:3 >4 process_order worker.py:1 >2 store_order :4",
        "{text}"
    );
    assert!(!text.contains("order_total >"), "{text}");
    assert_eq!(text, find(&root, &[])?, "repeat runs are byte-identical");

    let json = find(&root, &["--json"])?;
    let record = json
        .lines()
        .find(|line| line.contains("\"t\":\"chain\""))
        .ok_or_else(|| format!("no chain record: {json}"))?;
    assert!(
        record.contains("worker.py:1 process_order site 4 exact"),
        "{record}"
    );

    let tight = find(&root, &["-b", "20"])?;
    assert!(tight.len() <= 60, "{tight}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn chains_cross_from_configuration_to_scripts() -> Result<(), Box<dyn Error>> {
    let root = support::exclusive_dir("proof-chains-artifacts")?;
    fs::create_dir_all(root.join(".github/workflows"))?;
    fs::write(
        root.join(".github/workflows/lint.yml"),
        "name: ci\non: push\njobs:\n  lint:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: npm run lint\n",
    )?;
    fs::write(
        root.join("package.json"),
        "{\n  \"name\": \"x\",\n  \"scripts\": {\n    \"lint\": \"eslint .\"\n  }\n}\n",
    )?;
    let output = support::command(BINARY, &root)
        .args(["find", "lint", "script", "workflow", "-C"])
        .arg(&root)
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    // Cross-artifact link is visible either as a ~ chain or as inline < runs edge
    // (ConfigYAML ghost symbols flood leading hits after TASK 054, hiding the chain).
    assert!(
        text.lines().any(|line| line.starts_with("~ ")
            && line.contains(".github/workflows/lint.yml")
            && line.contains("package.json"))
            || text.contains(".github/workflows/lint.yml")
                && text.contains("package.json")
                && (text.contains("< lint ") || text.contains("runs")),
        "{text}"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
