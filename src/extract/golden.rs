//! Golden fact tests: every `tests/fixtures/languages/<id>/<case>.<ext>` is extracted and
//! compared with `<case>.<ext>.facts`. `XCALYGRAPH_BLESS=1` rewrites the golden files; every
//! blessed line must be checked against the fixture source before commit.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::corpus::FileRecord;
use crate::domain::facts::Facts;
use crate::domain::role::{self, FileFacts};
use crate::extract::extract;

/// Dense text form of facts, one fact per line.
pub fn render(facts: &Facts) -> String {
    let mut out = String::new();
    let scope = |index: Option<u32>| {
        index
            .and_then(|index| facts.symbols.get(index as usize))
            .map_or("-".to_owned(), |symbol| symbol.name.clone())
    };
    if let Some(package) = &facts.package {
        let _ = writeln!(out, "package {package}");
    }
    for symbol in &facts.symbols {
        let _ = writeln!(
            out,
            "sym {}-{} {} {} parent={} owner={} {} flags={} sig={:?} doc={:?}",
            symbol.line,
            symbol.end_line,
            symbol.kind().id(),
            symbol.name,
            scope(symbol.parent),
            symbol.owner,
            if symbol.exported { "pub" } else { "priv" },
            symbol.flags,
            symbol.signature,
            symbol.doc,
        );
    }
    for import in &facts.imports {
        let _ = writeln!(
            out,
            "import {} {:?} names={} alias={} reexport={}",
            import.line,
            import.spec,
            import.names.join(","),
            import.alias.as_deref().unwrap_or("-"),
            import.reexport,
        );
    }
    for reference in &facts.refs {
        let _ = writeln!(
            out,
            "ref {} {:?} {} qualifier={} scope={}",
            reference.line,
            reference.kind,
            reference.name,
            reference.qualifier.as_deref().unwrap_or("-"),
            scope(reference.scope),
        );
    }
    for binding in &facts.bindings {
        let _ = writeln!(
            out,
            "bind {} {:?}: {} {} scope={}",
            binding.line,
            binding.name,
            binding.type_name,
            if binding.declared {
                "declared"
            } else {
                "inferred"
            },
            scope(binding.scope),
        );
    }
    for fields in &facts.fields {
        let pairs: Vec<String> = fields
            .names
            .iter()
            .zip(&fields.types)
            .map(|(name, kind)| format!("{name}:{kind}"))
            .collect();
        let _ = writeln!(
            out,
            "fields {} {}",
            scope(Some(fields.owner)),
            pairs.join(" ")
        );
    }
    for env in &facts.envs {
        let _ = writeln!(
            out,
            "env {} {} scope={}",
            env.line,
            env.name,
            scope(env.scope)
        );
    }
    out
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/languages")
}

fn cases(dir: &Path) -> Vec<PathBuf> {
    let mut cases = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return cases;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            cases.extend(self::cases(&path));
        } else if path.extension().is_some_and(|ext| ext != "facts") {
            cases.push(path);
        }
    }
    cases.sort();
    cases
}

#[test]
fn golden_facts_match() -> Result<(), Box<dyn std::error::Error>> {
    let bless = std::env::var_os("XCALYGRAPH_BLESS").is_some();
    let root = fixtures();
    let mut failures = Vec::new();
    let all = cases(&root);
    assert!(!all.is_empty(), "no fixtures under {}", root.display());
    for case in all {
        let content = fs::read(&case)?;
        let relative = case
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        // The first directory is the language id; the rest is the path seen by the classifier.
        let path = relative
            .split_once('/')
            .map_or(relative.as_str(), |(_, rest)| rest);
        let classification = role::classify(&FileFacts {
            path,
            size: content.len() as u64,
            head: Some(&content[..content.len().min(role::HEAD_LEN)]),
            cpp_sibling: false,
        });
        let record = FileRecord {
            path: path.to_owned(),
            size: content.len() as u64,
            mtime_ns: 0,
            hash: 0,
            role: classification.role,
            file_type: classification.file_type,
        };
        let actual = render(&extract(&record, &content));
        let golden = PathBuf::from(format!("{}.facts", case.display()));
        if bless {
            fs::write(&golden, &actual)?;
            continue;
        }
        let expected = fs::read_to_string(&golden).unwrap_or_default();
        if expected != actual {
            failures.push(format!(
                "{relative}\n--- expected\n{expected}--- actual\n{actual}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatches:\n{}",
        failures.join("\n")
    );
    Ok(())
}
