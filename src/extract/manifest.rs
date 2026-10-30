//! Manifest facts: module names, dependencies, workspace members and entrypoints
//! (`docs/spec.md` section 5, manifest).

use crate::domain::facts::Manifest;

/// Parses a manifest by file name; `None` for manifests without a parser.
pub fn parse(name: &str, text: &str) -> Option<Manifest> {
    match name {
        "go.mod" => Some(go_mod(text)),
        "go.work" => Some(go_work(text)),
        "Cargo.toml" => Some(cargo_toml(text)),
        _ => None,
    }
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
    let mut bin_path: Option<String> = None;
    let mut bin_name: Option<String> = None;
    let flush_bin =
        |manifest: &mut Manifest, path: &mut Option<String>, name: &mut Option<String>| {
            if let Some(path) = path.take() {
                manifest.entrypoints.push(path);
            } else if let Some(name) = name.take() {
                manifest.entrypoints.push(format!("src/bin/{name}.rs"));
            }
            *name = None;
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
            if section == "[bin]" {
                flush_bin(&mut manifest, &mut bin_path, &mut bin_name);
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
        if section == "[bin]" {
            match key.as_str() {
                "path" => bin_path = Some(unquote(&value)),
                "name" => bin_name = Some(unquote(&value)),
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
    if section == "[bin]" {
        flush_bin(&mut manifest, &mut bin_path, &mut bin_name);
    }
    manifest
}

fn assign(manifest: &mut Manifest, section: &str, key: &str, value: &str) {
    match (section, key) {
        ("package", "name") => manifest.module = Some(unquote(value)),
        ("workspace", "members") => manifest.members.extend(array(value)),
        ("lib", "path") => manifest.source_roots.push(unquote(value)),
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
    use super::*;

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
    }
}
