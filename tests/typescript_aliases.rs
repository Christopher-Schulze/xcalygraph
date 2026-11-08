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
fn deno_import_maps_do_not_select_a_different_package_entry() -> TestResult {
    let root = support::exclusive_dir("alias-deno-map")?;
    let noise: String = (0..405)
        .map(|number| format!("\"noise{number}\":0,"))
        .collect();
    let config = format!(
        r#"{{/* map */{noise}"imports":{{"shared":"./dist/bundle.js","shared/":"./other/"}},"compilerOptions":{{"paths":{{"@/*":["./*"]}}}},}}"#
    );
    write(
        &root,
        &[
            (
                "package.json",
                r#"{"name":"shared","module":"./runtime.ts"}"#,
            ),
            ("runtime.ts", WHO),
            ("sub.ts", WHO),
            ("app/deno.jsonc", &config),
            ("app/tsconfig.json", r#"{"extends":"./deno.jsonc"}"#),
            ("app/api.ts", WHO),
            (
                "app/main.ts",
                "import { who } from 'shared';\nexport function denoEntry() { return who(); }\n",
            ),
            (
                "app/prefix.ts",
                "import { who } from 'shared/sub';\nexport function prefixEntry() { return who(); }\n",
            ),
            (
                "app/tsconfig-entry.ts",
                "import { who } from '@/api';\nexport function tsconfigEntry() { return who(); }\n",
            ),
            (
                "other/main.ts",
                "import { who } from 'shared';\nexport function nodeEntry() { return who(); }\n",
            ),
        ],
    )?;
    let mapped = show(&root, "denoEntry")?;
    assert!(!mapped.contains("> who runtime.ts"), "{mapped}");
    let prefix = show(&root, "prefixEntry")?;
    assert!(!prefix.contains("> who sub.ts"), "{prefix}");
    let inherited = show(&root, "tsconfigEntry")?;
    assert!(inherited.contains("> who app/api.ts:1"), "{inherited}");
    let other = show(&root, "nodeEntry")?;
    assert!(other.contains("> who runtime.ts:1"), "{other}");
    fs::write(
        root.join("app/deno.jsonc"),
        r#"{"imports":{"other":"./x.ts"}}"#,
    )?;
    let unrelated = show(&root, "denoEntry")?;
    assert!(unrelated.contains("> who runtime.ts:1"), "{unrelated}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn deno_scopes_external_maps_and_relative_keys_do_not_prove_targets() -> TestResult {
    let root = support::exclusive_dir("alias-deno-uncertain")?;
    write(
        &root,
        &[
            (
                "package.json",
                r#"{"name":"shared","module":"./runtime.ts"}"#,
            ),
            ("runtime.ts", WHO),
            ("app/thing.ts", WHO),
            (
                "app/main.ts",
                "import { who as one } from 'shared';\nimport { who as two } from './thing.ts';\nexport function denoEntry() { return one() + two(); }\n",
            ),
            (
                "app/deno.json",
                r#"{"scopes":{"./":{"shared":"./other.ts"}}}"#,
            ),
        ],
    )?;
    let scoped = show(&root, "denoEntry")?;
    assert!(!scoped.contains("> who runtime.ts"), "{scoped}");
    assert!(scoped.contains("> who app/thing.ts:1"), "{scoped}");
    fs::write(root.join("app/deno.json"), r#"{"importMap":"./map.json"}"#)?;
    let external = show(&root, "denoEntry")?;
    assert!(!external.contains("> who runtime.ts"), "{external}");
    assert!(!external.contains("> who app/thing.ts"), "{external}");
    fs::write(
        root.join("app/deno.json"),
        r#"{"imports":{"./thing.ts":"./other.ts"}}"#,
    )?;
    let relative = show(&root, "denoEntry")?;
    assert!(!relative.contains("> who app/thing.ts"), "{relative}");
    assert!(relative.contains("> who runtime.ts:1"), "{relative}");
    fs::write(root.join("app/deno.json"), "{ invalid")?;
    let invalid = show(&root, "denoEntry")?;
    assert!(!invalid.contains("> who runtime.ts"), "{invalid}");
    assert!(!invalid.contains("> who app/thing.ts"), "{invalid}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn uncertain_workspace_symbols_are_only_inferred_at_the_direct_reference() -> TestResult {
    let root = support::exclusive_dir("alias-deno-workspace-symbol")?;
    write(
        &root,
        &[
            (
                "package.json",
                r#"{"name":"shared","module":"./runtime.ts"}"#,
            ),
            (
                "runtime.ts",
                "export function who() { return 1; }\nfunction hidden() {}\nexport class Widget { run() {} }\n",
            ),
            (
                "app/package.json",
                r#"{"name":"app","devDependencies":{"shared":"workspace:*"}}"#,
            ),
            ("app/deno.json", r#"{"imports":{"shared":"./other.ts"}}"#),
            (
                "app/main.ts",
                "import { who, hidden, Widget } from 'shared';\nexport function entry() { return who(); }\nexport function shadow(who: () => number) { return who(); }\nexport function unexported() { hidden(); }\nexport function construct() { return new Widget(); }\nexport function bareClass() { return Widget(); }\nexport function typed(value: Widget) { value.run(); }\n",
            ),
            (
                "app/type-only.ts",
                "import type { Widget } from 'shared';\nexport function invalidConstructor() { return new Widget(); }\n",
            ),
        ],
    )?;
    let output = support::command(BINARY, &root)
        .args(["show", "entry", "--json", "-C"])
        .arg(&root)
        .output()?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(
        text.lines()
            .any(|line| line.contains("\"path\":\"runtime.ts\"")
                && line.contains("\"kind\":\"calls\"")
                && line.contains("\"conf\":\"inferred\"")),
        "{text}"
    );
    let exact = support::command(BINARY, &root)
        .args(["show", "entry", "--json", "--exact", "-C"])
        .arg(&root)
        .output()?;
    assert!(exact.status.success());
    assert!(!String::from_utf8(exact.stdout)?.contains("runtime.ts"));
    for name in ["shadow", "unexported", "bareClass", "invalidConstructor"] {
        let text = show(&root, name)?;
        assert!(!text.contains("runtime.ts"), "{name}: {text}");
    }
    for (name, kind) in [("construct", "calls"), ("typed", "uses")] {
        let output = support::command(BINARY, &root)
            .args(["show", name, "--json", "-C"])
            .arg(&root)
            .output()?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        assert!(
            text.lines()
                .any(|line| line.contains("\"path\":\"runtime.ts\"")
                    && line.contains(&format!("\"kind\":\"{kind}\""))
                    && line.contains("\"conf\":\"inferred\"")),
            "{name}: {text}"
        );
        assert!(
            !text.lines().any(
                |line| line.contains("\"name\":\"run\"") && line.contains("\"conf\":\"exact\"")
            ),
            "{name}: {text}"
        );
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_deno_conflicting_workspace_import_keeps_inferred_edges_only() -> TestResult {
    let root = support::exclusive_dir("alias-deno-workspace")?;
    write(
        &root,
        &[
            (
                "package.json",
                r#"{"name":"shared","module":"./runtime.ts"}"#,
            ),
            ("runtime.ts", WHO),
            (
                "app/package.json",
                r#"{"name":"app","devDependencies":{"shared":"workspace:*"}}"#,
            ),
            ("app/deno.json", r#"{"imports":{"shared":"./other.ts"}}"#),
            (
                "app/main.ts",
                "import { who } from 'shared';\nexport function denoEntry() { return who(); }\n",
            ),
        ],
    )?;
    let file = show(&root, "app/main.ts")?;
    assert!(
        file.lines()
            .any(|line| line.contains("runtime.ts:1") && line.contains("imports?")),
        "{file}"
    );
    let symbol = show(&root, "denoEntry")?;
    assert!(symbol.contains("> who runtime.ts:1?"), "{symbol}");
    let exact_symbol = support::command(BINARY, &root)
        .args(["show", "denoEntry", "--exact", "-C"])
        .arg(&root)
        .output()?;
    assert!(exact_symbol.status.success());
    assert!(!String::from_utf8(exact_symbol.stdout)?.contains("runtime.ts"));
    let exact = support::command(BINARY, &root)
        .args(["show", "app/main.ts", "--exact", "-C"])
        .arg(&root)
        .output()?;
    assert!(exact.status.success());
    assert!(!String::from_utf8(exact.stdout)?.contains("runtime.ts"));
    write(
        &root,
        &[
            (
                "app/tsconfig.json",
                r#"{"compilerOptions":{"paths":{"shared":["./alternate.ts"]}}}"#,
            ),
            ("app/alternate.ts", WHO),
        ],
    )?;
    let aliased = show(&root, "app/main.ts")?;
    assert!(
        !aliased.contains("runtime.ts") && !aliased.contains("alternate.ts"),
        "{aliased}"
    );
    fs::write(root.join("app/tsconfig.json"), "{}")?;
    fs::write(
        root.join("app/package.json"),
        r#"{"name":"app","devDependencies":{"shared":"^1"}}"#,
    )?;
    let external = show(&root, "app/main.ts")?;
    assert!(!external.contains("runtime.ts"), "{external}");
    fs::write(
        root.join("app/package.json"),
        r#"{"name":"app","devDependencies":{"shared":"workspace:*"}}"#,
    )?;
    write(&root, &[("outside.ts", WHO)])?;
    fs::write(
        root.join("package.json"),
        r#"{"name":"shared","module":"../outside.ts"}"#,
    )?;
    let outside = show(&root, "app/main.ts")?;
    assert!(!outside.contains("outside.ts"), "{outside}");
    fs::write(
        root.join("package.json"),
        r#"{"name":"shared","module":"./runtime.ts"}"#,
    )?;
    write(
        &root,
        &[
            (
                "duplicate/package.json",
                r#"{"name":"shared","module":"./runtime.ts"}"#,
            ),
            ("duplicate/runtime.ts", WHO),
        ],
    )?;
    let duplicate = show(&root, "app/main.ts")?;
    assert!(!duplicate.contains("runtime.ts"), "{duplicate}");
    fs::remove_dir_all(root)?;
    Ok(())
}

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
fn extends_follows_arbitrary_json_names_and_escaped_keys() -> TestResult {
    let root = support::exclusive_dir("alias-json-extends")?;
    let leading: String = (0..405)
        .map(|number| format!("\"noise{number}\":0,"))
        .collect();
    let base =
        format!(r#"{{{leading}"compiler\u004fptions":{{"paths":{{"@/*":["../shared/*",5]}}}}}}"#);
    write(
        &root,
        &[
            ("tsconfig.json", r#"{"extends":"./configs/middle.json"}"#),
            (
                "configs/middle.json",
                r#"{/* parent */"extends":"./base",}"#,
            ),
            ("configs/base.json", &base),
            ("shared/api.ts", WHO),
            ("other/api.ts", WHO),
            ("main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who shared/api.ts:1"), "{text}");
    fs::write(
        root.join("configs/base.json"),
        base.replace("../shared/*", "../other/*"),
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who other/api.ts:1"), "{text}");
    assert!(!text.contains("> who shared/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn package_json_can_be_extended_and_remain_a_package() -> TestResult {
    let root = support::exclusive_dir("alias-package-extends")?;
    write(
        &root,
        &[
            ("tsconfig.json", r#"{"extends":"./configs/package.json"}"#),
            (
                "configs/package.json",
                r#"{"name":"dual","main":"../runtime/api.ts","compilerOptions":{"paths":{"@=/*":["../shared/*"]}}}"#,
            ),
            ("shared/api.ts", WHO),
            ("runtime/api.ts", "export function pkg() { return 2; }\n"),
            (
                "main.ts",
                "import { who } from '@=/api';\nexport function entry() { return who(); }\n",
            ),
            (
                "other.ts",
                "import { pkg } from 'dual';\nexport function package_entry() { return pkg(); }\n",
            ),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who shared/api.ts:1"), "{text}");
    let text = show(&root, "package_entry")?;
    assert!(text.contains("> pkg runtime/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn an_unreferenced_json_file_does_not_become_a_project() -> TestResult {
    let root = support::exclusive_dir("alias-unreferenced-json")?;
    write(
        &root,
        &[
            (
                "base.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
            ),
            ("src/api.ts", WHO),
            ("main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn malformed_extended_json_does_not_supply_aliases() -> TestResult {
    for config in [
        r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}} trailing"#,
        r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}} /* unterminated"#,
    ] {
        let root = support::exclusive_dir("alias-malformed-json")?;
        write(
            &root,
            &[
                ("tsconfig.json", r#"{"extends":"./base.json"}"#),
                ("base.json", config),
                ("src/api.ts", WHO),
                ("main.ts", CALL),
            ],
        )?;
        let text = show(&root, "entry")?;
        assert!(!text.contains("> who"), "{config}: {text}");
        fs::remove_dir_all(root)?;
    }
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
fn references_select_a_project_with_an_arbitrary_json_name() -> TestResult {
    let root = support::exclusive_dir("alias-json-references")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"files":[],"references":[{"path":"./app.json"},{"path":"./node.json"}]}"#,
            ),
            (
                "app.json",
                r#"{"include":["src"],"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
            ),
            (
                "node.json",
                r#"{"include":["tools"],"compilerOptions":{"paths":{"@/*":["./tools/*"]}}}"#,
            ),
            ("src/api.ts", WHO),
            ("src/main.ts", CALL),
            ("tools/api.ts", WHO),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who src/api.ts:1"), "{text}");
    assert!(!text.contains("> who tools/api.ts:1"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn an_empty_referenced_project_does_not_inherit_the_root_aliases() -> TestResult {
    let root = support::exclusive_dir("alias-empty-project")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"files":[],"references":[{"path":"./app.json"}],"compilerOptions":{"paths":{"@/*":["./wrong/*"]}}}"#,
            ),
            ("app.json", r#"{"unrelated":true}"#),
            ("wrong/api.ts", WHO),
            ("main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_referenced_project_with_an_unproven_parent_blocks_project_selection() -> TestResult {
    let root = support::exclusive_dir("alias-unproven-reference")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"references":[{"path":"./app.json"}],"compilerOptions":{"paths":{"@/*":["./wrong/*"]}}}"#,
            ),
            ("app.json", r#"{"extends":"./missing.json"}"#),
            ("wrong/api.ts", WHO),
            ("main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn nested_project_references_do_not_prove_an_ancestor_alias() -> TestResult {
    let root = support::exclusive_dir("alias-nested-reference")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"references":[{"path":"./app.json"}],"compilerOptions":{"paths":{"@/*":["./wrong/*"]}}}"#,
            ),
            ("app.json", r#"{"references":[{"path":"./leaf.json"}]}"#),
            ("leaf.json", "{}"),
            ("wrong/api.ts", WHO),
            ("main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_missing_or_malformed_parent_does_not_prove_child_aliases() -> TestResult {
    for parent in [None, Some("{} trailing")] {
        let root = support::exclusive_dir("alias-invalid-parent")?;
        write(
            &root,
            &[
                (
                    "tsconfig.json",
                    r#"{"extends":"./parent.json","compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
                ),
                ("src/api.ts", WHO),
                ("main.ts", CALL),
            ],
        )?;
        if let Some(parent) = parent {
            fs::write(root.join("parent.json"), parent)?;
        }
        let text = show(&root, "entry")?;
        assert!(!text.contains("> who"), "{parent:?}: {text}");
        fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn an_absolute_or_outside_parent_never_resolves_to_an_in_root_decoy() -> TestResult {
    for parent in ["/outside/base.json", "../../outside/base.json"] {
        let root = support::exclusive_dir("alias-outside-parent")?;
        let config = format!(r#"{{"extends":"{parent}"}}"#);
        write(
            &root,
            &[
                ("tsconfig.json", &config),
                (
                    "outside/base.json",
                    r#"{"compilerOptions":{"paths":{"@/*":["../shared/*"]}}}"#,
                ),
                ("shared/api.ts", WHO),
                ("main.ts", CALL),
            ],
        )?;
        let text = show(&root, "entry")?;
        assert!(!text.contains("> who"), "{parent}: {text}");
        fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn outside_alias_targets_never_resolve_to_in_root_decoys_or_fallbacks() -> TestResult {
    for config in [
        r#"{"compilerOptions":{"paths":{"@/*":["../../outside/*"]}}}"#,
        r#"{"compilerOptions":{"baseUrl":"../../outside","paths":{"@/*":["*"]}}}"#,
        r#"{"compilerOptions":{"paths":{"@/*":["../../missing/*","./safe/*"]}}}"#,
    ] {
        let root = support::exclusive_dir("alias-outside-target")?;
        write(
            &root,
            &[
                ("tsconfig.json", r#"{"extends":"./base.json"}"#),
                ("base.json", config),
                ("outside/api.ts", WHO),
                ("safe/api.ts", WHO),
                ("main.ts", CALL),
            ],
        )?;
        let text = show(&root, "entry")?;
        assert!(!text.contains("> who"), "{config}: {text}");
        fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn an_unproven_outside_alias_does_not_fall_back_to_an_in_root_package() -> TestResult {
    let root = support::exclusive_dir("alias-outside-package")?;
    write(
        &root,
        &[
            ("tsconfig.json", r#"{"extends":"./base.json"}"#),
            (
                "base.json",
                r#"{"compilerOptions":{"paths":{"@scope/*":["../../outside/*"]}}}"#,
            ),
            (
                "package.json",
                r#"{"name":"@scope/pkg","main":"./fake.ts"}"#,
            ),
            ("fake.ts", WHO),
            (
                "main.ts",
                "import { who } from '@scope/pkg';\nexport function entry() { return who(); }\n",
            ),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn an_unproven_parent_does_not_select_a_same_named_local_package() -> TestResult {
    let root = support::exclusive_dir("alias-unproven-package")?;
    write(
        &root,
        &[
            ("tsconfig.json", r#"{"extends":"./missing.json"}"#),
            (
                "package.json",
                r#"{"name":"@scope/pkg","main":"./fake.ts"}"#,
            ),
            ("fake.ts", WHO),
            (
                "main.ts",
                "import { who } from '@scope/pkg';\nexport function entry() { return who(); }\n",
            ),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_malformed_nearest_project_does_not_use_a_parent_project() -> TestResult {
    let root = support::exclusive_dir("alias-malformed-nearest")?;
    write(
        &root,
        &[
            (
                "tsconfig.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./wrong/*"]}}}"#,
            ),
            ("wrong/api.ts", WHO),
            ("app/tsconfig.json", "{ invalid"),
            ("app/main.ts", CALL),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(!text.contains("> who"), "{text}");
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
fn svelte_lib_remains_available_without_the_generated_project_config() -> TestResult {
    let root = support::exclusive_dir("alias-svelte-generated")?;
    let call = "import { who } from '$lib/api';\nexport function entry() { return who(); }\n";
    write(
        &root,
        &[
            ("svelte.config.js", "export default {};\n"),
            (
                "jsconfig.json",
                r#"{"extends":"./.svelte-kit/tsconfig.json"}"#,
            ),
            ("src/lib/api.js", WHO),
            ("src/main.js", call),
        ],
    )?;
    let text = show(&root, "entry")?;
    assert!(text.contains("> who src/lib/api.js:1"), "{text}");
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
