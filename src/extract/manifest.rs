//! Manifest facts: module names, dependencies, workspace members and entrypoints
//! (`docs/spec.md` section 5, manifest).

use super::build_manifests;
use super::json::{self, Value};
use crate::domain::facts::{Manifest, RustTargetRoot};

/// Parses a manifest by file name; `None` for manifests without a parser.
pub fn parse(name: &str, text: &str) -> Option<Manifest> {
    match name {
        "go.mod" => Some(go_mod(text)),
        "go.work" => Some(go_work(text)),
        "Cargo.toml" => Some(cargo_toml(text)),
        "package.json" => Some(package_json(&json::parse(text)?)),
        "tsconfig.json" | "jsconfig.json" => Some(tsconfig(&json::parse(text)?)),
        "CMakeLists.txt" => Some(cmake(text)),
        "compile_commands.json" => Some(compile_commands(&json::parse(text)?)),
        "DESCRIPTION" => Some(r_description(text)),
        "pubspec.yaml" => Some(pubspec(text)),
        "remappings.txt" | "foundry.toml" => Some(solidity_remappings(text)),
        "pyproject.toml" => Some(build_manifests::pyproject(text)),
        "pom.xml" => Some(build_manifests::pom(text)),
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => {
            Some(build_manifests::gradle(name, text))
        }
        "Package.swift" => Some(build_manifests::swift_package(text)),
        "composer.json" => Some(build_manifests::composer(&json::parse(text)?)),
        "Gemfile" => Some(build_manifests::ruby(text)),
        "mix.exs" => Some(build_manifests::mix(text)),
        _ if name.ends_with(".gemspec") => Some(build_manifests::ruby(text)),
        _ if [".csproj", ".vbproj", ".fsproj"]
            .iter()
            .any(|suffix| name.ends_with(suffix)) =>
        {
            Some(build_manifests::dotnet_project(name, text))
        }
        _ if name.ends_with(".sln") || name.ends_with(".slnx") => {
            Some(build_manifests::solution(text))
        }
        _ => None,
    }
}

/// `name`, dependency keys, `workspaces`, `bin` entrypoints and `main`/`module` as source root.
fn package_json(value: &Value) -> Manifest {
    let mut manifest = Manifest {
        module: value.get("name").and_then(Value::as_str).map(str::to_owned),
        ..Manifest::default()
    };
    for section in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        for (name, _, requirement) in value.get(section).map(Value::entries).unwrap_or_default() {
            if !manifest.deps.iter().any(|(existing, _)| existing == name) {
                manifest.deps.push((
                    name.clone(),
                    requirement.as_str().unwrap_or_default().to_owned(),
                ));
            }
        }
    }
    if let Some(workspaces) = value.get("workspaces") {
        manifest.members = match workspaces.get("packages") {
            Some(packages) => packages.strings(),
            None => workspaces.strings(),
        };
    }
    match value.get("bin") {
        Some(Value::Str(path)) => manifest.entrypoints.push(path.clone()),
        Some(bin) => manifest.entrypoints.extend(
            bin.entries()
                .iter()
                .filter_map(|(_, _, path)| path.as_str().map(str::to_owned)),
        ),
        None => {}
    }
    for key in ["module", "main"] {
        if let Some(main) = value.get(key).and_then(Value::as_str) {
            manifest.source_roots.push(main.to_owned());
            break;
        }
    }
    manifest
}

/// `compilerOptions.baseUrl` as source root, `paths` as `alias=target` include entries.
fn tsconfig(value: &Value) -> Manifest {
    let mut manifest = Manifest::default();
    let Some(options) = value.get("compilerOptions") else {
        return manifest;
    };
    if let Some(base) = options.get("baseUrl").and_then(Value::as_str) {
        manifest.source_roots.push(base.to_owned());
    }
    for (alias, _, targets) in options.get("paths").map(Value::entries).unwrap_or_default() {
        if let Some(target) = targets.strings().first() {
            manifest.include_dirs.push(format!("{alias}={target}"));
        }
    }
    manifest
}

/// R `DESCRIPTION`: `Package` name, `Depends`, `Imports` and `LinkingTo` packages (`R` itself
/// skipped), continuation lines indented.
fn r_description(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let mut field = String::new();
    for line in text.lines() {
        let value = if line.starts_with([' ', '\t']) {
            line
        } else {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            field = key.trim().to_owned();
            value
        };
        match field.as_str() {
            "Package" if manifest.module.is_none() => {
                manifest.module = Some(value.trim().to_owned()).filter(|name| !name.is_empty());
            }
            "Depends" | "Imports" | "LinkingTo" => {
                for entry in value.split(',') {
                    let (name, requirement) = entry.split_once('(').unwrap_or((entry, ""));
                    let name = name.trim();
                    if !name.is_empty() && name != "R" {
                        let requirement = requirement.trim().trim_end_matches(')').trim();
                        manifest
                            .deps
                            .push((name.to_owned(), requirement.to_owned()));
                    }
                }
            }
            _ => {}
        }
    }
    manifest
}

/// Solidity import remappings (`prefix=target`, `remappings.txt` lines or quoted entries of
/// `foundry.toml`) as dependencies: name = prefix, requirement = target directory.
fn solidity_remappings(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    for line in text.lines() {
        let entries: Vec<&str> = if line.contains('"') {
            line.split('"').skip(1).step_by(2).collect()
        } else {
            vec![line.trim()]
        };
        for entry in entries {
            // Context-specific remappings: `context:prefix=target`.
            let entry = entry.rsplit_once(':').map_or(entry, |(_, rest)| rest);
            if let Some((prefix, target)) = entry.split_once('=')
                && !prefix.trim().is_empty()
                && !prefix.contains(' ')
            {
                manifest
                    .deps
                    .push((prefix.trim().to_owned(), target.trim().to_owned()));
            }
        }
    }
    manifest
}

/// Dart `pubspec.yaml`: package `name` and the keys of `dependencies` and `dev_dependencies`.
fn pubspec(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let mut section = "";
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indented = line.starts_with(' ');
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        if !indented {
            section = if matches!(key, "dependencies" | "dev_dependencies" | "executables") {
                key
            } else {
                ""
            };
            if key == "name" {
                manifest.module = Some(value.trim().to_owned()).filter(|name| !name.is_empty());
            }
        } else if section == "executables" && line.starts_with("  ") {
            // `name:` runs `bin/name.dart`, `name: script` runs `bin/script.dart`.
            let script = Some(value.trim())
                .filter(|script| !script.is_empty())
                .unwrap_or(key);
            manifest.entrypoints.push(format!("bin/{script}.dart"));
        } else if !section.is_empty() && line.starts_with("  ") && !line.starts_with("   ") {
            manifest
                .deps
                .push((key.to_owned(), value.trim().to_owned()));
        }
    }
    manifest
}

fn go_mod(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let mut in_require = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if let Some(module) = line.strip_prefix("module ") {
            manifest.module = Some(module.trim().trim_matches('"').to_owned());
        } else if line.starts_with("require (") {
            in_require = true;
        } else if in_require && line == ")" {
            in_require = false;
        } else if let Some(rest) = line.strip_prefix("require ").or(in_require.then_some(line)) {
            let mut parts = rest.split_whitespace();
            if let (Some(name), Some(version)) = (parts.next(), parts.next()) {
                manifest.deps.push((name.to_owned(), version.to_owned()));
            }
        }
    }
    manifest
}

fn go_work(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let mut in_use = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if line.starts_with("use (") {
            in_use = true;
        } else if in_use && line == ")" {
            in_use = false;
        } else if let Some(dir) = line.strip_prefix("use ").or(in_use.then_some(line))
            && !dir.is_empty()
        {
            manifest
                .members
                .push(dir.trim_start_matches("./").to_owned());
        }
    }
    manifest
}

/// Minimal TOML reading: sections, `key = value` pairs and multi-line arrays; enough for
/// package names, dependency keys, workspace members and targets.
fn cargo_toml(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let mut section = String::new();
    let mut pending: Option<(String, String)> = None;
    let mut target_path: Option<String> = None;
    let mut target_name: Option<String> = None;
    let flush_target = |manifest: &mut Manifest,
                        section: &str,
                        path: &mut Option<String>,
                        name: &mut Option<String>| {
        let Some((directory, binary)) = cargo_target_section(section) else {
            return;
        };
        let target = path
            .take()
            .or_else(|| name.take().map(|name| format!("{directory}/{name}.rs")));
        *name = None;
        if let Some(path) = target {
            manifest.rust_target_roots.push(RustTargetRoot {
                path: path.clone(),
                is_library: false,
            });
            if binary {
                manifest.entrypoints.push(path);
            }
        }
    };
    for raw in text.lines() {
        let line = strip_toml_comment(raw).trim().to_owned();
        if let Some((key, mut value)) = pending.take() {
            value.push_str(&line);
            if line.contains(']') {
                assign(&mut manifest, &section, &key, &value);
            } else {
                pending = Some((key, value));
            }
            continue;
        }
        if line.starts_with('[') {
            if cargo_target_section(&section).is_some() {
                flush_target(&mut manifest, &section, &mut target_path, &mut target_name);
            }
            section = line
                .trim_matches(|ch| ch == '[' || ch == ']')
                .trim()
                .to_owned();
            if line.starts_with("[[") {
                section = format!("[{section}]");
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (
            key.trim().trim_matches('"').to_owned(),
            value.trim().to_owned(),
        );
        if cargo_target_section(&section).is_some() {
            match key.as_str() {
                "path" => target_path = Some(unquote(&value)),
                "name" => target_name = Some(unquote(&value)),
                _ => {}
            }
            continue;
        }
        if value.starts_with('[') && !value.contains(']') {
            pending = Some((key, value));
            continue;
        }
        assign(&mut manifest, &section, &key, &value);
    }
    if cargo_target_section(&section).is_some() {
        flush_target(&mut manifest, &section, &mut target_path, &mut target_name);
    }
    manifest
}

fn cargo_target_section(section: &str) -> Option<(&'static str, bool)> {
    match section {
        "[bin]" => Some(("src/bin", true)),
        "[example]" => Some(("examples", false)),
        "[test]" => Some(("tests", false)),
        "[bench]" => Some(("benches", false)),
        _ => None,
    }
}

/// `include_directories(...)` and `target_include_directories(target [SYSTEM] [BEFORE] PUBLIC
/// dir ...)` arguments relative to the `CMakeLists.txt`; source-dir variables stripped, other
/// variables skipped. `project(name)` names the module.
fn cmake(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let flat: String = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ");
    let lower = flat.to_ascii_lowercase();
    for command in [
        "include_directories",
        "target_include_directories",
        "project",
    ] {
        let mut from = 0;
        while let Some(found) = lower[from..].find(&format!("{command}(")) {
            let start = from + found + command.len() + 1;
            let Some(length) = flat[start..].find(')') else {
                break;
            };
            let args: Vec<&str> = flat[start..start + length].split_whitespace().collect();
            let boundary = lower[..from + found]
                .chars()
                .next_back()
                .is_none_or(|before| !(before.is_alphanumeric() || before == '_'));
            from = start + length;
            if !boundary {
                continue;
            }
            if command == "project" {
                manifest.module = manifest
                    .module
                    .take()
                    .or_else(|| args.first().map(|name| (*name).to_owned()));
                continue;
            }
            let skip = usize::from(command == "target_include_directories");
            for arg in args.into_iter().skip(skip) {
                if matches!(
                    arg,
                    "PUBLIC" | "PRIVATE" | "INTERFACE" | "SYSTEM" | "BEFORE" | "AFTER"
                ) {
                    continue;
                }
                let arg = arg.trim_matches('"');
                let arg = [
                    "${CMAKE_CURRENT_SOURCE_DIR}",
                    "${PROJECT_SOURCE_DIR}",
                    "${CMAKE_SOURCE_DIR}",
                ]
                .iter()
                .find_map(|prefix| arg.strip_prefix(prefix))
                .map_or(arg, |rest| rest.trim_start_matches('/'));
                if arg.contains("${") || arg.contains("$<") {
                    continue;
                }
                let dir = if arg.is_empty() { "." } else { arg };
                if !manifest.include_dirs.iter().any(|existing| existing == dir) {
                    manifest.include_dirs.push(dir.to_owned());
                }
            }
        }
    }
    manifest
}

/// `-I` and `-isystem` directories of every compile command, as written (usually absolute; the
/// resolver maps them onto root directories by suffix).
fn compile_commands(value: &Value) -> Manifest {
    let mut manifest = Manifest::default();
    let Value::Array(entries) = value else {
        return manifest;
    };
    for entry in entries {
        let arguments: Vec<String> = match entry.get("arguments") {
            Some(arguments) => arguments.strings(),
            None => entry
                .get("command")
                .and_then(Value::as_str)
                .map(|command| command.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
        };
        let mut words = arguments.iter().peekable();
        while let Some(word) = words.next() {
            let dir = if let Some(dir) = word.strip_prefix("-I").filter(|dir| !dir.is_empty()) {
                Some(dir.to_owned())
            } else if word == "-I" || word == "-isystem" {
                words.next().cloned()
            } else {
                word.strip_prefix("-isystem")
                    .filter(|dir| !dir.is_empty())
                    .map(str::to_owned)
            };
            if let Some(dir) = dir
                && !manifest.include_dirs.contains(&dir)
            {
                manifest.include_dirs.push(dir);
            }
        }
    }
    manifest
}

fn assign(manifest: &mut Manifest, section: &str, key: &str, value: &str) {
    match (section, key) {
        ("package", "name") => manifest.module = Some(unquote(value)),
        ("workspace", "members") => manifest.members.extend(array(value)),
        ("lib", "path") => {
            let path = unquote(value);
            manifest.source_roots.push(path.clone());
            manifest.rust_target_roots.push(RustTargetRoot {
                path,
                is_library: true,
            });
        }
        (section, name)
            if section == "dependencies"
                || section.ends_with(".dependencies")
                || section == "dev-dependencies"
                || section == "build-dependencies"
                || section == "workspace.dependencies" =>
        {
            let requirement = if value.starts_with('{') {
                value
                    .split("version")
                    .nth(1)
                    .and_then(|rest| rest.split('"').nth(1))
                    .unwrap_or_default()
                    .to_owned()
            } else {
                unquote(value)
            };
            if !manifest.deps.iter().any(|(existing, _)| existing == name) {
                manifest.deps.push((name.to_owned(), requirement));
            }
        }
        _ => {}
    }
}

fn array(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(unquote)
        .filter(|item| !item.is_empty())
        .collect()
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .to_owned()
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_string = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..index],
            _ => {}
        }
    }
    line
}

#[cfg(test)]
mod tests {
    #[test]
    fn r_description_and_pubspec() {
        let description = parse(
            "DESCRIPTION",
            "Package: store\nVersion: 1.0\nDepends: R (>= 4.0), methods\nImports:\n    dplyr (>= 1.1),\n    rlang\n",
        );
        assert_eq!(
            description.map(|m| (m.module, m.deps)),
            Some((
                Some("store".to_owned()),
                vec![
                    ("methods".to_owned(), String::new()),
                    ("dplyr".to_owned(), ">= 1.1".to_owned()),
                    ("rlang".to_owned(), String::new())
                ]
            ))
        );
        let pubspec = parse(
            "pubspec.yaml",
            "name: app\nenvironment:\n  sdk: ^3.0.0\ndependencies:\n  http: ^1.2.0\n  path:\n    git: x\ndev_dependencies:\n  test: any\n",
        );
        assert_eq!(
            pubspec.map(|m| (m.module, m.deps.len())),
            Some((Some("app".to_owned()), 3))
        );
    }

    use super::*;

    #[test]
    fn cmake_and_compile_commands_list_include_directories() {
        let cmake = parse(
            "CMakeLists.txt",
            "project(demo C)\ninclude_directories(include ${CMAKE_CURRENT_SOURCE_DIR}/third_party)\ntarget_include_directories(app PUBLIC src ${GEN_DIR}) # generated\n",
        );
        assert_eq!(
            cmake
                .as_ref()
                .map(|manifest| (manifest.module.clone(), manifest.include_dirs.clone())),
            Some((
                Some("demo".to_owned()),
                vec![
                    "include".to_owned(),
                    "third_party".to_owned(),
                    "src".to_owned()
                ]
            ))
        );
        let commands = parse(
            "compile_commands.json",
            r#"[{"directory": "/b", "command": "cc -Iinclude -I /w/lib/inc -c a.c", "file": "a.c"}, {"arguments": ["cc", "-isystem/w/sys"]}]"#,
        );
        assert_eq!(
            commands.map(|manifest| manifest.include_dirs),
            Some(vec![
                "include".to_owned(),
                "/w/lib/inc".to_owned(),
                "/w/sys".to_owned()
            ])
        );
    }

    #[test]
    fn package_json_and_tsconfig() {
        let package = parse(
            "package.json",
            "{\"name\": \"@acme/web\", \"main\": \"dist/index.js\", \"bin\": {\"acme\": \"bin/acme.js\"}, \"workspaces\": {\"packages\": [\"packages/*\"]}, \"dependencies\": {\"react\": \"^19\"}}",
        );
        let package = package.unwrap_or_default();
        assert_eq!(package.module.as_deref(), Some("@acme/web"));
        assert_eq!(package.members, ["packages/*"]);
        assert_eq!(package.entrypoints, ["bin/acme.js"]);
        assert_eq!(package.deps, [("react".to_owned(), "^19".to_owned())]);
        assert_eq!(package.source_roots, ["dist/index.js"]);
        let config = parse(
            "tsconfig.json",
            "{ // jsonc\n \"compilerOptions\": { \"baseUrl\": \".\", \"paths\": { \"@/*\": [\"src/*\"] }, }, }",
        )
        .unwrap_or_default();
        assert_eq!(config.source_roots, ["."]);
        assert_eq!(config.include_dirs, ["@/*=src/*"]);
    }

    #[test]
    fn go_mod_names_module_and_requirements() {
        let manifest = go_mod(
            "module example.com/app // main\n\ngo 1.23\n\nrequire (\n\tgithub.com/a/b v1.2.0\n\tgolang.org/x/c v0.1.0 // indirect\n)\nrequire d.io/e v2.0.0\n",
        );
        assert_eq!(manifest.module.as_deref(), Some("example.com/app"));
        assert_eq!(
            manifest.deps,
            [
                ("github.com/a/b".to_owned(), "v1.2.0".to_owned()),
                ("golang.org/x/c".to_owned(), "v0.1.0".to_owned()),
                ("d.io/e".to_owned(), "v2.0.0".to_owned()),
            ]
        );
        let work = go_work("go 1.23\nuse (\n\t./api\n\t./worker\n)\nuse ./tools\n");
        assert_eq!(work.members, ["api", "worker", "tools"]);
    }

    #[test]
    fn cargo_toml_names_package_members_deps_and_bins() {
        let manifest = cargo_toml(
            "[package]\nname = \"my-app\" # comment\n\n[workspace]\nmembers = [\n  \"crates/core\",\n  \"crates/cli\",\n]\n\n[dependencies]\nserde = { version = \"1.0\", features = [\"derive\"] }\nrayon = \"1.12\"\n\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n\n[[bin]]\nname = \"tool\"\n\n[[bin]]\nname = \"other\"\npath = \"src/other.rs\"\n",
        );
        assert_eq!(manifest.module.as_deref(), Some("my-app"));
        assert_eq!(manifest.members, ["crates/core", "crates/cli"]);
        assert_eq!(
            manifest.deps,
            [
                ("serde".to_owned(), "1.0".to_owned()),
                ("rayon".to_owned(), "1.12".to_owned()),
                ("libc".to_owned(), "0.2".to_owned()),
            ]
        );
        assert_eq!(manifest.entrypoints, ["src/bin/tool.rs", "src/other.rs"]);
        assert_eq!(
            manifest.rust_target_roots,
            [
                RustTargetRoot {
                    path: "src/bin/tool.rs".to_owned(),
                    is_library: false,
                },
                RustTargetRoot {
                    path: "src/other.rs".to_owned(),
                    is_library: false,
                },
            ]
        );
    }

    #[test]
    fn cargo_toml_records_custom_library_and_all_target_roots() {
        let manifest = cargo_toml(
            "[package]\nname = \"custom-root\"\n\n[lib]\npath = \"src/custom.rs\"\n\n[[bin]]\nname = \"tool\"\npath = \"tools/cli.rs\"\n\n[[example]]\nname = \"default-example\"\n\n[[example]]\nname = \"demo\"\npath = \"samples/demo.rs\"\n\n[[test]]\nname = \"default-test\"\n\n[[test]]\nname = \"integration\"\npath = \"checks/integration.rs\"\n\n[[bench]]\nname = \"default-bench\"\n\n[[bench]]\nname = \"speed\"\npath = \"benchmarks/speed.rs\"\n",
        );
        assert_eq!(manifest.source_roots, ["src/custom.rs"]);
        assert_eq!(manifest.entrypoints, ["tools/cli.rs"]);
        assert_eq!(
            manifest.rust_target_roots,
            [
                RustTargetRoot {
                    path: "src/custom.rs".to_owned(),
                    is_library: true,
                },
                RustTargetRoot {
                    path: "tools/cli.rs".to_owned(),
                    is_library: false,
                },
                RustTargetRoot {
                    path: "examples/default-example.rs".to_owned(),
                    is_library: false,
                },
                RustTargetRoot {
                    path: "samples/demo.rs".to_owned(),
                    is_library: false,
                },
                RustTargetRoot {
                    path: "tests/default-test.rs".to_owned(),
                    is_library: false,
                },
                RustTargetRoot {
                    path: "checks/integration.rs".to_owned(),
                    is_library: false,
                },
                RustTargetRoot {
                    path: "benches/default-bench.rs".to_owned(),
                    is_library: false,
                },
                RustTargetRoot {
                    path: "benchmarks/speed.rs".to_owned(),
                    is_library: false,
                },
            ]
        );
    }
}
