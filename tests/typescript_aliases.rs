//! Path aliases resolve inside the project of the importing file, as `tsc` selects them.

mod support;

use std::error::Error;
use std::fs;
use std::path::Path;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

type TestResult = Result<(), Box<dyn Error>>;

fn write(root: &Path, files: &[(&str, &str)]) -> TestResult {
    for (path, content) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(())
}

fn show(root: &Path, name: &str) -> Result<String, Box<dyn Error>> {
    let output = support::command(BINARY, root)
        .args(["show", name, "-C"])
        .arg(root)
        .output()?;
    assert!(output.status.success(), "{output:?}");
    Ok(String::from_utf8(output.stdout)?)
}

const CALL: &str = "import { who } from \"@/api\";\nexport function entry() { return who(); }\n";
const WHO: &str = "export function who() { return 1; }\n";

#[test]
fn each_app_uses_its_own_alias_table() -> TestResult {
    let root = support::exclusive_dir("alias-apps")?;
    let config = r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#;
    write(
        &root,
        &[
            ("apps/a/tsconfig.json", config),
            ("apps/a/src/api.ts", WHO),
            ("apps/z/tsconfig.json", config),
            ("apps/z/src/api.ts", WHO),
            ("apps/z/src/main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who apps/z/src/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_foreign_config_never_answers() -> TestResult {
    let root = support::exclusive_dir("alias-foreign")?;
    write(
        &root,
        &[
            (
                "apps/a/tsconfig.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
            ),
            ("apps/a/src/api.ts", WHO),
            ("apps/b/src/main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn extends_inherits_paths_relative_to_the_declaring_config() -> TestResult {
    let root = support::exclusive_dir("alias-extends")?;
    write(
        &root,
        &[
            (
                "tsconfig.base.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./shared/*"]}}}"#,
            ),
            ("shared/api.ts", WHO),
            (
                "pkg/tsconfig.json",
                r#"{"extends":"../tsconfig.base.json"}"#,
            ),
            ("pkg/main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who shared/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn child_paths_replace_the_parents() -> TestResult {
    let root = support::exclusive_dir("alias-replace")?;
    write(
        &root,
        &[
            (
                "tsconfig.base.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./shared/*"]}}}"#,
            ),
            ("shared/api.ts", WHO),
            (
                "pkg/tsconfig.json",
                r#"{"extends":"../tsconfig.base.json","compilerOptions":{"paths":{"~/*":["./src/*"]}}}"#,
            ),
            ("pkg/main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn references_select_the_project_holding_the_file() -> TestResult {
    let root = support::exclusive_dir("alias-references")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"files":[],"references":[{"path":"./tsconfig.app.json"},{"path":"./tsconfig.node.json"}]}"#,
            ),
            (
                "tsconfig.app.json",
                r#"{"include":["src"],"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
            ),
            (
                "tsconfig.node.json",
                r#"{"include":["tools"],"compilerOptions":{"paths":{"@/*":["./tools/*"]}}}"#,
            ),
            ("src/api.ts", WHO),
            ("src/main.ts", CALL),
            ("tools/api.ts", WHO),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who src/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn later_targets_are_fallbacks_in_array_order() -> TestResult {
    let root = support::exclusive_dir("alias-fallback")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./missing/*","./z/*","./a/*"]}}}"#,
            ),
            ("a/api.ts", WHO),
            ("z/api.ts", WHO),
            ("main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who z/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn the_longest_prefix_wins_without_falling_back_to_a_shorter_alias() -> TestResult {
    let root = support::exclusive_dir("alias-longest")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./a/*"],"@/lib/*":["./missing/*"]}}}"#,
            ),
            ("a/lib/x.ts", WHO),
            (
                "main.ts",
                "import { who } from \"@/lib/x\";\nexport function entry() { return who(); }\n",
            ),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn an_exact_alias_matches_only_the_whole_specifier() -> TestResult {
    let root = support::exclusive_dir("alias-exact")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"compilerOptions":{"paths":{"api":["./a/api.ts"],"api*":["./z/*"]}}}"#,
            ),
            ("a/api.ts", WHO),
            ("z/.ts", WHO),
            (
                "main.ts",
                "import { who } from \"api\";\nexport function entry() { return who(); }\n",
            ),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who a/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn lib_alias_stays_inside_its_svelte_app() -> TestResult {
    let root = support::exclusive_dir("alias-lib")?;
    let call = "import { who } from \"$lib/api\";\nexport function entry() { return who(); }\n";
    write(
        &root,
        &[
            ("a/svelte.config.js", "export default {};\n"),
            ("a/src/lib/api.ts", WHO),
            ("z/svelte.config.js", "export default {};\n"),
            ("z/src/lib/api.ts", WHO),
            ("z/src/main.ts", call),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who z/src/lib/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn jsconfig_answers_where_no_tsconfig_exists() -> TestResult {
    let root = support::exclusive_dir("alias-jsconfig")?;
    write(
        &root,
        &[
            (
                "jsconfig.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
            ),
            ("src/api.js", WHO),
            ("main.js", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who src/api.js:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}
