//! A natural-language question can name distinct operations in the same file.

mod support;

use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> io::Result<Self> {
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".cache");
        fs::create_dir_all(&cache)?;
        if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
            return Err(io::Error::other("test cache must not be a symlink"));
        }
        for sequence in 0..1000 {
            let path = cache.join(format!(
                "search-diversity-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("no unused test directory"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn different_query_evidence_in_one_file_survives_duplicate_siblings() -> Result<(), Box<dyn Error>>
{
    let work = TempDir::new()?;
    let root = work.0.join("root");
    fs::create_dir(&root)?;
    let mut source = String::new();
    for name in ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"] {
        source.push_str(&format!(
            "pub fn compress_archive_{name}() {{\n    let _ = \"compress archive\";\n}}\n\n"
        ));
    }
    source.push_str("pub fn decompress_archive() {\n    let _ = \"decompress archive\";\n}\n");
    fs::write(root.join("archive.rs"), source)?;
    for number in 0..12 {
        fs::write(
            root.join(format!("other_{number}.rs")),
            "pub fn decompress_related() {\n    let _ = \"decompress\";\n}\n",
        )?;
    }
    for verb in ["find", "ctx"] {
        for json in [false, true] {
            let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0);
            command
                .args([verb, "compress and decompress archive", "-b", "600", "-C"])
                .arg(&root)
                .arg("--index-dir")
                .arg(work.0.join("index"))
                .env("HOME", &work.0);
            if json {
                command.arg("--json");
            }
            let output = command.output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(
                answer.contains("compress_archive_alpha"),
                "{verb} json={json}: {answer}"
            );
            assert!(
                answer.contains("decompress_archive"),
                "{verb} json={json}: {answer}"
            );
            assert!(
                answer.contains("decompress_related"),
                "{verb} json={json}: {answer}"
            );
        }
    }
    Ok(())
}

#[test]
fn related_word_forms_find_definitions_without_hiding_exact_matches() -> Result<(), Box<dyn Error>>
{
    let work = TempDir::new()?;
    let root = work.0.join("root");
    fs::create_dir(&root)?;
    fs::write(
        root.join("operations.rs"),
        "pub fn validate(ok: bool) -> bool { ok }\n\
         pub fn validation_report(count: usize) -> usize { count }\n\
         pub fn tokenize(text: &str) -> usize { text.split_whitespace().count() }\n\
         pub fn get(items: &[u8]) -> Option<u8> { items.first().copied() }\n",
    )?;
    for (question, expected) in [
        ("validation", "fn validate("),
        ("token", "fn tokenize("),
        ("retrieval", "fn get("),
    ] {
        for verb in ["find", "ctx"] {
            let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0)
                .args([verb, question, "-b", "600", "-C"])
                .arg(&root)
                .arg("--index-dir")
                .arg(work.0.join("index"))
                .env("HOME", &work.0)
                .output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(answer.contains(expected), "{verb} {question}: {answer}");
            if verb == "find" && question == "validation" {
                assert!(
                    answer.contains("validation_report")
                        && answer.find("validation_report") < answer.find("fn validate("),
                    "exact match should precede the expansion: {answer}"
                );
            }
        }
    }
    let many_terms = (100..170)
        .map(|number| number.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let long_question = format!("validation {many_terms}");
    for verb in ["find", "ctx"] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0)
            .args([verb, &long_question, "-b", "600", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(work.0.join("index"))
            .env("HOME", &work.0)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let answer = String::from_utf8(output.stdout)?;
        assert!(answer.contains("validation_report"), "{verb}: {answer}");
    }
    Ok(())
}

#[test]
fn context_covers_distinct_named_concepts_before_repeating_one() -> Result<(), Box<dyn Error>> {
    let work = TempDir::new()?;
    let root = work.0.join("root");
    fs::create_dir(&root)?;
    for number in 0..8 {
        fs::write(
            root.join(format!("writer_{number}.rs")),
            format!("pub fn write_image_{number}(data: &[u8]) -> usize {{\n    data.len()\n}}\n"),
        )?;
    }
    fs::write(
        root.join("checks.rs"),
        "pub fn validate(data: &[u8]) -> bool {\n    !data.is_empty()\n}\n",
    )?;
    for json in [false, true] {
        let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0);
        command
            .args(["ctx", "writing image and validation", "-b", "600", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(work.0.join("index"))
            .env("HOME", &work.0);
        if json {
            command.arg("--json");
        }
        let output = command.output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let answer = String::from_utf8(output.stdout)?;
        assert!(answer.contains("write_image_0"), "json={json}: {answer}");
        assert!(answer.contains("validate"), "json={json}: {answer}");
    }
    Ok(())
}

#[test]
fn context_is_byte_identical_across_processes_for_a_symmetric_graph() -> Result<(), Box<dyn Error>>
{
    let work = TempDir::new()?;
    let root = work.0.join("root");
    fs::create_dir(&root)?;
    let mut source = String::from("pub fn start() { ");
    for number in 0..8 {
        for side in ["left", "right"] {
            source.push_str(&format!("branch_{side}_{number}(); "));
        }
    }
    source.push_str("}\n");
    for side in ["left", "right"] {
        source.push_str(&format!("fn {side}() -> usize {{ 1 }}\n"));
        for number in 0..8 {
            source.push_str(&format!("fn branch_{side}_{number}() {{ {side}(); "));
            for leaf in 0..number {
                source.push_str(&format!("leaf_{side}_{number}_{leaf}(); "));
            }
            source.push_str("}\n");
            for leaf in 0..number {
                source.push_str(&format!(
                    "fn leaf_{side}_{number}_{leaf}() -> usize {{ {} }}\n",
                    leaf + 1
                ));
            }
        }
    }
    fs::write(root.join("graph.rs"), source)?;
    let index = work.0.join("index");
    let built = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0)
        .args(["index", "-C"])
        .arg(&root)
        .arg("--index-dir")
        .arg(&index)
        .env("HOME", &work.0)
        .output()?;
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    for json in [false, true] {
        let mut previous = None;
        for _ in 0..20 {
            let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0);
            command
                .args(["ctx", "start", "-b", "600", "-C"])
                .arg(&root)
                .arg("--index-dir")
                .arg(&index)
                .arg("--read-only")
                .env("HOME", &work.0);
            if json {
                command.arg("--json");
            }
            let output = command.output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(
                answer.contains("start") && answer.contains("branch_"),
                "{answer}"
            );
            if let Some(previous) = &previous {
                assert_eq!(previous, &answer, "json={json}");
            }
            previous = Some(answer);
        }
    }
    Ok(())
}

#[test]
fn find_is_byte_identical_for_symbols_on_the_same_line() -> Result<(), Box<dyn Error>> {
    let work = TempDir::new()?;
    let root = work.0.join("root");
    fs::create_dir(&root)?;
    fs::write(
        root.join("operations.rs"),
        "pub fn read_alpha() -> usize { 1 } pub fn read_beta() -> usize { 2 }\n",
    )?;
    for json in [false, true] {
        let mut previous = None;
        for _ in 0..20 {
            let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0);
            command
                .args(["find", "read", "-b", "600", "-C"])
                .arg(&root)
                .arg("--index-dir")
                .arg(work.0.join("index"))
                .env("HOME", &work.0);
            if json {
                command.arg("--json");
            }
            let output = command.output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(
                answer.contains("read_alpha") && answer.contains("read_beta"),
                "{answer}"
            );
            if let Some(previous) = &previous {
                assert_eq!(previous, &answer, "json={json}");
            }
            previous = Some(answer);
        }
    }
    Ok(())
}

#[test]
fn filtered_symbol_search_keeps_terms_inherited_from_its_file_path() -> Result<(), Box<dyn Error>> {
    let work = TempDir::new()?;
    let root = work.0.join("root");
    fs::create_dir_all(root.join("src"))?;
    fs::write(
        root.join("src/alpha.rs"),
        "pub fn load_entry() -> usize { 1 }\npub fn save_entry() -> usize { 2 }\n",
    )?;
    fs::write(
        root.join("src/beta.rs"),
        "pub fn load_entry() -> usize { 3 }\n",
    )?;
    for json in [false, true] {
        let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0);
        command
            .args([
                "find",
                "alpha entry",
                "-p",
                "src/alpha.rs",
                "-k",
                "fn",
                "-C",
            ])
            .arg(&root)
            .arg("--index-dir")
            .arg(work.0.join("index"))
            .env("HOME", &work.0);
        if json {
            command.arg("--json");
        }
        let output = command.output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let answer = String::from_utf8(output.stdout)?;
        assert!(
            answer.contains("load_entry") && answer.contains("save_entry"),
            "{answer}"
        );
        assert!(!answer.contains("beta.rs"), "{answer}");
    }
    Ok(())
}

#[test]
fn exact_names_keep_their_bonus_for_ascii_and_unicode() -> Result<(), Box<dyn Error>> {
    let work = TempDir::new()?;
    let root = work.0.join("root");
    fs::create_dir(&root)?;
    fs::write(
        root.join("operations.rs"),
        "pub fn Read_Entry() -> usize { 1 }\n\
         pub fn read_entry_other() -> usize { 2 }\n\
         pub fn Café_report() -> usize { 3 }\n\
         pub fn café_report_other() -> usize { 4 }\n",
    )?;
    for (question, expected) in [("READ-ENTRY", "Read_Entry"), ("CAFÉ_REPORT", "Café_report")] {
        let output = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0)
            .args(["find", question, "-b", "600", "--json", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(work.0.join("index"))
            .env("HOME", &work.0)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let answer = String::from_utf8(output.stdout)?;
        let first = answer.lines().next().ok_or("missing search result")?;
        assert!(first.contains(expected), "{question}: {answer}");
        let score: f32 = first
            .split_once("\"score\":")
            .ok_or("missing search score")?
            .1
            .trim_end_matches('}')
            .parse()?;
        assert!(score > 5.0, "exact-name bonus missing: {answer}");
    }
    Ok(())
}

#[test]
fn explicit_exact_symbol_queries_reject_semantic_neighbors_and_keep_behavioral_search()
-> Result<(), Box<dyn Error>> {
    let work = TempDir::new()?;
    let root = work.0.join("root");
    fs::create_dir_all(root.join("src"))?;
    fs::write(
        root.join("src/lib.rs"),
        "pub mod accounts;\npub mod auth;\npub mod billing;\n",
    )?;
    fs::write(
        root.join("src/accounts.rs"),
        "pub fn validate_request() -> bool { false }\n",
    )?;
    fs::write(
        root.join("src/auth.rs"),
        "pub fn validate_request() -> bool { true }\n",
    )?;
    fs::write(
        root.join("src/billing.rs"),
        "pub struct BillingClient;\n\
         pub fn invoice_billing_client_config() {}\n\
         pub fn payment_billing_client_config() {}\n\
         pub fn authorize_invoice_before_creation() -> bool { true }\n\
         pub fn validate_invoice() {}\n\
         /// Performs billing persistence.\n\
         pub fn persist_billing_client() {}\n",
    )?;
    let index = work.0.join("index");
    let run = |verb: &str, query: &str, json: bool| {
        let mut command = support::command(env!("CARGO_BIN_EXE_xcalygraph"), &work.0);
        command
            .args([verb, query, "-b", "600", "-C"])
            .arg(&root)
            .arg("--index-dir")
            .arg(&index);
        if json {
            command.arg("--json");
        }
        command.output()
    };

    for query in [
        "where is the exact symbol `InvoiceBillingClient` defined?",
        "WHERE IS THE EXACT SYMBOL PaymentBillingClient defined?",
    ] {
        for verb in ["find", "ctx"] {
            for json in [false, true] {
                let output = run(verb, query, json)?;
                assert_eq!(
                    output.status.code(),
                    Some(4),
                    "{verb} json={json} query={query}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                assert!(
                    output.stdout.is_empty(),
                    "{verb} json={json} query={query}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
        }
    }

    for verb in ["find", "ctx"] {
        for json in [false, true] {
            let output = run(
                verb,
                "where is the exact symbol `BillingClient` defined",
                json,
            )?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(answer.contains("BillingClient"), "{answer}");
            if verb == "find" {
                assert!(
                    !answer.contains("invoice_billing_client_config"),
                    "{answer}"
                );
                assert!(
                    !answer.contains("payment_billing_client_config"),
                    "{answer}"
                );
            } else {
                let exact = answer
                    .find("BillingClient")
                    .ok_or("missing exact ctx seed")?;
                for related in [
                    "invoice_billing_client_config",
                    "payment_billing_client_config",
                ] {
                    if let Some(related) = answer.find(related) {
                        assert!(exact < related, "exact seed must precede context: {answer}");
                    }
                }
            }

            let output = run(
                verb,
                "where is the exact symbol auth::validate_request defined",
                json,
            )?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(answer.contains("src/auth.rs"), "{answer}");
            if verb == "find" {
                assert!(!answer.contains("src/accounts.rs"), "{answer}");
            } else if let Some(accounts) = answer.find("src/accounts.rs") {
                assert!(
                    answer
                        .find("src/auth.rs")
                        .is_some_and(|auth| auth < accounts),
                    "qualified exact seed must precede unrelated context: {answer}"
                );
            }

            let output = run(
                verb,
                "where is the exact symbol validate_request defined",
                json,
            )?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(answer.contains("src/auth.rs"), "{answer}");
            assert!(answer.contains("src/accounts.rs"), "{answer}");

            let output = run(verb, "how is an invoice authorized before creation", json)?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(
                answer.contains("authorize_invoice_before_creation"),
                "{answer}"
            );

            let output = run(
                verb,
                "where are invoice validation and billing persistence implemented",
                json,
            )?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer = String::from_utf8(output.stdout)?;
            assert!(answer.contains("validate_invoice"), "{answer}");
            assert!(answer.contains("persist_billing_client"), "{answer}");
        }
    }

    Ok(())
}
