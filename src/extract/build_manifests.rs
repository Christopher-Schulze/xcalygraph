//! Manifests of the JVM, .NET, Python, Swift, PHP, Ruby and Elixir build tools (`docs/spec.md`
//! section 5, manifest): module name, dependencies, members and declared entrypoints. Line and
//! tag scanners, no build tool evaluation: values computed by build scripts are skipped.

use crate::domain::facts::Manifest;

/// `pyproject.toml`: `[project]` name, `dependencies`, optional dependencies and
/// `[project.scripts]` (`tool = "pkg.cli:main"` names `pkg/cli.py`, also under `src/`); Poetry
/// `[tool.poetry]` name, dependencies and scripts.
pub fn pyproject(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let mut section = String::new();
    let mut pending: Option<String> = None;
    for raw in text.lines() {
        let line = raw.split(" #").next().unwrap_or(raw).trim();
        if let Some(list) = pending.as_mut() {
            list.push_str(line);
            if line.contains(']') {
                for requirement in quoted_items(list) {
                    push_python_dep(&mut manifest, &requirement);
                }
                pending = None;
            }
            continue;
        }
        if line.starts_with('[') {
            section = line
                .trim_matches(|ch| ch == '[' || ch == ']')
                .trim()
                .to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim().trim_matches('"'), value.trim());
        match section.as_str() {
            "project" | "tool.poetry" if key == "name" => {
                manifest.module = Some(unquote(value)).filter(|name| !name.is_empty());
            }
            "project" if key == "dependencies" => {
                if value.contains(']') {
                    for requirement in quoted_items(value) {
                        push_python_dep(&mut manifest, &requirement);
                    }
                } else {
                    pending = Some(value.to_owned());
                }
            }
            "project.optional-dependencies" => {
                if value.contains(']') {
                    for requirement in quoted_items(value) {
                        push_python_dep(&mut manifest, &requirement);
                    }
                } else {
                    pending = Some(value.to_owned());
                }
            }
            "tool.poetry.dependencies" | "tool.poetry.dev-dependencies" if key != "python" => {
                push_dep(&mut manifest, key, &unquote(value));
            }
            "project.scripts" | "project.gui-scripts" | "tool.poetry.scripts" => {
                let target = unquote(value);
                let module = target.split(':').next().unwrap_or(&target);
                let path = module.replace('.', "/");
                manifest.entrypoints.push(format!("{path}.py"));
                manifest.entrypoints.push(format!("src/{path}.py"));
                manifest.entrypoints.push(format!("{path}/__main__.py"));
                manifest.entrypoints.push(format!("src/{path}/__main__.py"));
            }
            _ => {}
        }
    }
    manifest
}

/// PEP 508 requirement `name[extra]>=1.0; marker`: the name and the version part.
fn push_python_dep(manifest: &mut Manifest, requirement: &str) {
    let requirement = requirement.split(';').next().unwrap_or(requirement).trim();
    let end = requirement
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
        .unwrap_or(requirement.len());
    let (name, rest) = requirement.split_at(end);
    let version = rest
        .trim_start_matches(|ch: char| {
            ch == '[' || ch.is_ascii_alphanumeric() || ch == ',' || ch == ']'
        })
        .trim();
    push_dep(manifest, name, version);
}

/// Gradle `build.gradle(.kts)`: `group`/`rootProject.name`-free scan of dependency notations
/// (`implementation("g:a:v")`, `api 'g:a:v'`), `project(":lib")` dependencies as members, and
/// `mainClass` of the `application` plugin as entrypoint sources (Java and Kotlin layouts).
/// `settings.gradle(.kts)`: `include(":a", ":b")` members and `rootProject.name`.
pub fn gradle(name: &str, text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    const CONFIGURATIONS: [&str; 10] = [
        "implementation",
        "api",
        "compileOnly",
        "runtimeOnly",
        "testImplementation",
        "testRuntimeOnly",
        "androidTestImplementation",
        "kapt",
        "ksp",
        "annotationProcessor",
    ];
    let settings = name.starts_with("settings.gradle");
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("//") {
            continue;
        }
        if settings {
            if line.starts_with("rootProject.name") {
                manifest.module = quoted_items(line).into_iter().next();
            } else if line.starts_with("include") {
                for member in quoted_items(line) {
                    manifest
                        .members
                        .push(member.trim_start_matches(':').replace(':', "/"));
                }
            }
            continue;
        }
        if line.contains("mainClass") {
            if let Some(class) = quoted_items(line).into_iter().next() {
                let path = class.replace('.', "/");
                for (dir, extension) in [("java", "java"), ("kotlin", "kt"), ("groovy", "groovy")] {
                    manifest
                        .entrypoints
                        .push(format!("src/main/{dir}/{path}.{extension}"));
                }
                // Kotlin top-level `main` lives in `MainKt` for `Main.kt`.
                if let Some(stem) = path.strip_suffix("Kt") {
                    manifest
                        .entrypoints
                        .push(format!("src/main/kotlin/{stem}.kt"));
                }
            }
            continue;
        }
        let dependency = CONFIGURATIONS.iter().any(|configuration| {
            line.strip_prefix(*configuration)
                .is_some_and(|rest| rest.starts_with(['(', ' ', '\'', '"']))
        });
        if !dependency {
            continue;
        }
        if line.contains("project(") {
            for member in quoted_items(line) {
                manifest
                    .members
                    .push(member.trim_start_matches(':').replace(':', "/"));
            }
            continue;
        }
        for notation in quoted_items(line) {
            let mut parts = notation.split(':');
            if let (Some(group), Some(artifact)) = (parts.next(), parts.next()) {
                let version = parts.next().unwrap_or_default();
                push_dep(&mut manifest, &format!("{group}:{artifact}"), version);
            }
        }
    }
    manifest
}

/// Maven `pom.xml`: the project's own `artifactId` (outside `<parent>`, `<dependencies>`,
/// `<build>`), dependencies as `groupId:artifactId` with version, `<modules>` members, and a
/// `mainClass` (exec, jar or shade plugin configuration) as entrypoint source.
pub fn pom(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let outer = remove_blocks(
        text,
        &[
            "parent",
            "dependencies",
            "dependencyManagement",
            "build",
            "profiles",
            "reporting",
        ],
    );
    manifest.module = tag_values(&outer, "artifactId").into_iter().next();
    for dependency in tag_values(text, "dependency") {
        let group = tag_values(&dependency, "groupId")
            .into_iter()
            .next()
            .unwrap_or_default();
        let Some(artifact) = tag_values(&dependency, "artifactId").into_iter().next() else {
            continue;
        };
        let version = tag_values(&dependency, "version")
            .into_iter()
            .next()
            .unwrap_or_default();
        push_dep(&mut manifest, &format!("{group}:{artifact}"), &version);
    }
    for modules in tag_values(text, "modules") {
        manifest.members.extend(tag_values(&modules, "module"));
    }
    for class in tag_values(text, "mainClass") {
        let path = class.trim().replace('.', "/");
        manifest
            .entrypoints
            .push(format!("src/main/java/{path}.java"));
        manifest
            .entrypoints
            .push(format!("src/main/kotlin/{path}.kt"));
    }
    manifest
}

/// `.csproj`, `.vbproj`, `.fsproj`: `AssemblyName` (else the file stem) as module,
/// `PackageReference` dependencies, `ProjectReference` paths as members, and `OutputType` `Exe`
/// or `WinExe` making `Program.cs`/`Program.vb`/`Program.fs` the entrypoint.
pub fn dotnet_project(name: &str, text: &str) -> Manifest {
    let mut manifest = Manifest {
        module: tag_values(text, "AssemblyName")
            .into_iter()
            .next()
            .or_else(|| name.rsplit_once('.').map(|(stem, _)| stem.to_owned())),
        ..Manifest::default()
    };
    for element in elements(text, "PackageReference") {
        if let Some(package) =
            attribute(&element, "Include").or_else(|| attribute(&element, "Update"))
        {
            let version = attribute(&element, "Version")
                .or_else(|| tag_values(&element, "Version").into_iter().next())
                .unwrap_or_default();
            push_dep(&mut manifest, &package, &version);
        }
    }
    for element in elements(text, "ProjectReference") {
        if let Some(path) = attribute(&element, "Include") {
            manifest.members.push(path.replace('\\', "/"));
        }
    }
    if tag_values(text, "OutputType")
        .iter()
        .any(|kind| matches!(kind.trim(), "Exe" | "WinExe"))
    {
        let program = match name.rsplit('.').next() {
            Some("vbproj") => "Program.vb",
            Some("fsproj") => "Program.fs",
            _ => "Program.cs",
        };
        manifest.entrypoints.push(program.to_owned());
    }
    manifest
}

/// `.sln` (`Project("{type}") = "Name", "path\to\x.csproj", ...`) and `.slnx`
/// (`<Project Path="..."/>`): the solution's projects as members.
pub fn solution(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    for line in text.lines() {
        if line.starts_with("Project(")
            && let Some(path) = quoted_items(line)
                .into_iter()
                .find(|item| item.ends_with("proj"))
        {
            manifest.members.push(path.replace('\\', "/"));
        }
    }
    for element in elements(text, "Project") {
        if let Some(path) = attribute(&element, "Path") {
            manifest.members.push(path.replace('\\', "/"));
        }
    }
    manifest
}

/// `Package.swift`: package `name:`, `.package(url: ".../name.git", ...)` dependencies, and
/// `.executableTarget(name: "X"` making `Sources/X/main.swift` the entrypoint.
pub fn swift_package(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let compact: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(rest) = compact.split("Package(").nth(1)
        && let Some(name) = rest
            .split("name:")
            .nth(1)
            .and_then(|value| quoted_items(value).into_iter().next())
    {
        manifest.module = Some(name);
    }
    for part in compact.split(".package(").skip(1) {
        let call = part.split(')').next().unwrap_or(part);
        let items = quoted_items(call);
        let Some(url) = items.first() else { continue };
        let dependency = url
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or(url)
            .to_owned();
        let version = items.get(1).cloned().unwrap_or_default();
        push_dep(&mut manifest, &dependency, &version);
    }
    for part in compact.split(".executableTarget(").skip(1) {
        if let Some(name) = part
            .split("name:")
            .nth(1)
            .and_then(|value| quoted_items(value).into_iter().next())
        {
            manifest
                .entrypoints
                .push(format!("Sources/{name}/main.swift"));
        }
    }
    manifest
}

/// `composer.json` is parsed as JSON by the caller; this reads `name`, `require`,
/// `require-dev` and `bin` from its pairs.
pub fn composer(value: &super::json::Value) -> Manifest {
    let mut manifest = Manifest {
        module: value
            .get("name")
            .and_then(super::json::Value::as_str)
            .map(str::to_owned),
        ..Manifest::default()
    };
    for section in ["require", "require-dev"] {
        for (name, _, requirement) in value
            .get(section)
            .map(super::json::Value::entries)
            .unwrap_or_default()
        {
            if name != "php" && !name.starts_with("ext-") {
                push_dep(
                    &mut manifest,
                    name,
                    requirement.as_str().unwrap_or_default(),
                );
            }
        }
    }
    if let Some(bin) = value.get("bin") {
        manifest.entrypoints.extend(bin.strings());
    }
    manifest
}

/// `Gemfile` (`gem "name", "~> 1.0"`) and `*.gemspec` (`add_dependency "name", "~> 1.0"`,
/// `spec.name = "x"`, `executables = ["x"]` naming `exe/x` and `bin/x`).
pub fn ruby(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        let dependency = line.starts_with("gem ")
            || line.contains("add_dependency")
            || line.contains("add_runtime_dependency")
            || line.contains("add_development_dependency");
        if dependency {
            let items = quoted_items(line);
            if let Some(name) = items.first() {
                let version = items.get(1).cloned().unwrap_or_default();
                push_dep(&mut manifest, name, &version);
            }
        } else if line.contains(".name") && line.contains('=') && manifest.module.is_none() {
            manifest.module = quoted_items(line).into_iter().next();
        } else if line.contains(".executables") && line.contains('=') {
            for executable in quoted_items(line) {
                manifest.entrypoints.push(format!("exe/{executable}"));
                manifest.entrypoints.push(format!("bin/{executable}"));
            }
        }
    }
    manifest
}

/// `mix.exs`: `app: :name` and `{:dep, "~> 1.0"}` tuples in `deps`.
pub fn mix(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    for raw in text.lines() {
        let line = raw.trim();
        // `app: :plug` inside `project/0` (also `[app: :plug,`), not `otp_app:`.
        if manifest.module.is_none()
            && let Some(position) = line.find("app:")
            && (position == 0 || matches!(line.as_bytes()[position - 1], b'[' | b' '))
        {
            let value = line[position + 4..].trim().trim_start_matches(':');
            let name: String = value
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            manifest.module = Some(name).filter(|name| !name.is_empty());
        }
        for tuple in line.split("{:").skip(1) {
            let tuple = tuple.split('}').next().unwrap_or(tuple);
            let name: String = tuple
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            if !name.is_empty() {
                let version = quoted_items(tuple).into_iter().next().unwrap_or_default();
                push_dep(&mut manifest, &name, &version);
            }
        }
    }
    manifest
}

fn push_dep(manifest: &mut Manifest, name: &str, version: &str) {
    if !name.is_empty() && !manifest.deps.iter().any(|(existing, _)| existing == name) {
        manifest.deps.push((name.to_owned(), version.to_owned()));
    }
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(',')
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .to_owned()
}

/// Every single- or double-quoted string of a line, in order.
fn quoted_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(['"', '\'']) {
        let quote = rest.as_bytes()[start] as char;
        let after = &rest[start + 1..];
        let Some(end) = after.find(quote) else {
            break;
        };
        items.push(after[..end].to_owned());
        rest = &after[end + 1..];
    }
    items
}

/// Text content of every `<tag>...</tag>` (no nesting of the same tag).
fn tag_values(text: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(&close) else {
            break;
        };
        values.push(after[..end].trim().to_owned());
        rest = &after[end + close.len()..];
    }
    values
}

/// The text with every `<tag>...</tag>` block of the given tags removed.
fn remove_blocks(text: &str, tags: &[&str]) -> String {
    let mut out = text.to_owned();
    for tag in tags {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        while let Some(start) = out.find(&open) {
            let Some(end) = out[start..]
                .find(&close)
                .map(|offset| start + offset + close.len())
            else {
                break;
            };
            out.replace_range(start..end, "");
        }
    }
    out
}

/// Every `<Name ...>` or `<Name .../>` element start tag, with its attributes, plus the inner
/// text up to `</Name>` when the element is not self-closing.
fn elements(text: &str, name: &str) -> Vec<String> {
    let open = format!("<{name} ");
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start..];
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let self_closing = after[..tag_end].ends_with('/');
        let end = if self_closing {
            tag_end + 1
        } else {
            after
                .find(&format!("</{name}>"))
                .map_or(tag_end + 1, |close| close + name.len() + 3)
        };
        found.push(after[..end].to_owned());
        rest = &after[end..];
    }
    found
}

/// Value of `name="..."` in an element start tag.
fn attribute(element: &str, name: &str) -> Option<String> {
    let head = element.split('>').next()?;
    let key = format!("{name}=\"");
    let start = head.find(&key)? + key.len();
    let end = head[start..].find('"')?;
    Some(head[start..start + end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(manifest: &Manifest) -> Vec<(&str, &str)> {
        manifest
            .deps
            .iter()
            .map(|(name, version)| (name.as_str(), version.as_str()))
            .collect()
    }

    #[test]
    fn pyproject_reads_name_dependencies_and_scripts() {
        let manifest = pyproject(
            "[project]\nname = \"acme\"\ndependencies = [\n  \"requests>=2.31\",\n  \"rich[jupyter]~=13.0; python_version>'3.8'\",\n]\n\n[project.scripts]\nacme = \"acme.cli:main\"\n",
        );
        assert_eq!(manifest.module.as_deref(), Some("acme"));
        assert_eq!(
            deps(&manifest),
            [("requests", ">=2.31"), ("rich", "~=13.0")]
        );
        assert!(manifest.entrypoints.contains(&"src/acme/cli.py".to_owned()));
    }

    #[test]
    fn gradle_reads_dependencies_members_and_main_class() {
        let manifest = gradle(
            "build.gradle.kts",
            "dependencies {\n  implementation(\"com.squareup.okio:okio:3.9.0\")\n  testImplementation 'junit:junit:4.13'\n  implementation(project(\":core\"))\n}\napplication {\n  mainClass.set(\"com.acme.MainKt\")\n}\n",
        );
        assert_eq!(
            deps(&manifest),
            [("com.squareup.okio:okio", "3.9.0"), ("junit:junit", "4.13")]
        );
        assert_eq!(manifest.members, ["core"]);
        assert!(
            manifest
                .entrypoints
                .contains(&"src/main/kotlin/com/acme/Main.kt".to_owned())
        );
        let settings = gradle(
            "settings.gradle",
            "rootProject.name = 'acme'\ninclude ':core', ':app:web'\n",
        );
        assert_eq!(settings.module.as_deref(), Some("acme"));
        assert_eq!(settings.members, ["core", "app/web"]);
    }

    #[test]
    fn pom_reads_own_artifact_dependencies_and_modules() {
        let manifest = pom(
            "<project><parent><artifactId>parent</artifactId></parent><artifactId>acme-core</artifactId><modules><module>api</module></modules><dependencies><dependency><groupId>com.google.guava</groupId><artifactId>guava</artifactId><version>33.0</version></dependency></dependencies></project>",
        );
        assert_eq!(manifest.module.as_deref(), Some("acme-core"));
        assert_eq!(deps(&manifest), [("com.google.guava:guava", "33.0")]);
        assert_eq!(manifest.members, ["api"]);
    }

    #[test]
    fn dotnet_projects_and_solutions() {
        let project = dotnet_project(
            "Web.csproj",
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup><ItemGroup><PackageReference Include=\"Serilog\" Version=\"3.1.1\" /><ProjectReference Include=\"..\\Core\\Core.csproj\" /></ItemGroup></Project>",
        );
        assert_eq!(project.module.as_deref(), Some("Web"));
        assert_eq!(deps(&project), [("Serilog", "3.1.1")]);
        assert_eq!(project.members, ["../Core/Core.csproj"]);
        assert_eq!(project.entrypoints, ["Program.cs"]);
        let sln = solution(
            "Project(\"{FAE04EC0}\") = \"Web\", \"src\\Web\\Web.csproj\", \"{1}\"\nEndProject\n",
        );
        assert_eq!(sln.members, ["src/Web/Web.csproj"]);
        let slnx = solution("<Solution><Project Path=\"src/Core/Core.csproj\" /></Solution>");
        assert_eq!(slnx.members, ["src/Core/Core.csproj"]);
    }

    #[test]
    fn swift_composer_ruby_and_mix() {
        let package = swift_package(
            "let package = Package(\n  name: \"Demo\",\n  dependencies: [.package(url: \"https://github.com/apple/swift-argument-parser.git\", from: \"1.3.0\")],\n  targets: [.executableTarget(name: \"demo\")]\n)",
        );
        assert_eq!(package.module.as_deref(), Some("Demo"));
        assert_eq!(deps(&package), [("swift-argument-parser", "1.3.0")]);
        assert_eq!(package.entrypoints, ["Sources/demo/main.swift"]);
        let gemfile = ruby("source 'https://rubygems.org'\ngem \"rack\", \"~> 3.0\"\ngem 'puma'\n");
        assert_eq!(deps(&gemfile), [("rack", "~> 3.0"), ("puma", "")]);
        let mix = mix(
            "def project do\n  [app: :plug,\n   deps: deps()]\nend\ndefp deps do\n  [{:mime, \"~> 2.0\"}, {:telemetry, \"~> 1.0\"}]\nend\n",
        );
        assert_eq!(mix.module.as_deref(), Some("plug"));
        assert_eq!(deps(&mix), [("mime", "~> 2.0"), ("telemetry", "~> 1.0")]);
    }
}
