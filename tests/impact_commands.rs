//! End-to-end checks that `impact` emits runnable test commands from the query root.

mod support;

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;

type TestResult = Result<(), Box<dyn Error>>;

const BINARY: &str = env!("CARGO_BIN_EXE_xcalygraph");

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> io::Result<Self> {
        let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache");
        fs::create_dir_all(&cache)?;
        if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
            return Err(io::Error::other("test cache must not be a symlink"));
        }
        let base = cache.canonicalize()?;
        let prefix = format!("xcalygraph-it-{}-{label}", std::process::id());
        for sequence in 0..1000 {
            let name = if sequence == 0 {
                prefix.clone()
            } else {
                format!("{prefix}-{sequence}")
            };
            let path = base.join(name);
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("no unused test directory"))
    }

    fn write(&self, root: &Path, relative: &str, content: &str) -> io::Result<()> {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn impact(root: &Path, index: &Path, fixture: &Path, target: &str) -> io::Result<Output> {
    support::command(BINARY, fixture)
        .args(["impact", target])
        .arg("-C")
        .arg(root)
        .arg("--index-dir")
        .arg(index)
        .output()
}

fn commands(output: &Output) -> Result<Vec<String>, Box<dyn Error>> {
    let text = String::from_utf8(output.stdout.clone())?;
    Ok(text
        .lines()
        .filter_map(|line| line.strip_prefix("r "))
        .map(decode_text_escapes)
        .collect::<Result<Vec<_>, _>>()?)
}

fn decode_text_escapes(text: &str) -> io::Result<String> {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        let Some(escape) = characters.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete text escape",
            ));
        };
        match escape {
            '\\' => output.push('\\'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'u' => {
                let mut codepoint = 0u32;
                for _ in 0..4 {
                    let Some(digit) = characters.next().filter(char::is_ascii_hexdigit) else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid text escape",
                        ));
                    };
                    let Some(value) = digit.to_digit(16) else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid text escape",
                        ));
                    };
                    codepoint = codepoint * 16 + value;
                }
                let Some(control) = char::from_u32(codepoint) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid text escape codepoint",
                    ));
                };
                output.push(control);
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unknown text escape",
                ));
            }
        }
    }
    Ok(output)
}

fn execute(root: &Path, fixture: &Path, command: &str) -> io::Result<Output> {
    let mut shell = support::command("sh", fixture);
    shell.arg("-c").arg(command).current_dir(root).output()
}

#[test]
fn nested_go_test_commands_run_from_the_query_root_with_spaces_and_quotes() -> TestResult {
    let fixture = TempDir::new("impact-go-commands")?;
    let root = fixture.0.join("query root");
    let module = "go module's $workspace";
    fixture.write(
        &root,
        &format!("{module}/go.mod"),
        "module example.com/storage\n\ngo 1.23\n",
    )?;
    fixture.write(
        &root,
        &format!("{module}/core/storage.go"),
        "package core\n\nfunc Load() int { return 1 }\n",
    )?;
    for (directory, package, function, test) in [
        ("pkg-a", "alpha", "UseA", "TestUseA"),
        ("pkg-b", "beta", "UseB", "TestUseB"),
    ] {
        fixture.write(
            &root,
            &format!("{module}/{directory}/use.go"),
            &format!(
                "package {package}\n\nimport \"example.com/storage/core\"\n\nfunc {function}() int {{ return core.Load() }}\n"
            ),
        )?;
        fixture.write(
            &root,
            &format!("{module}/{directory}/use_test.go"),
            &format!(
                "package {package}\n\nimport \"testing\"\n\nfunc {test}(t *testing.T) {{ if {function}() != 1 {{ t.Fatal() }} }}\n"
            ),
        )?;
    }

    let index = fixture.0.join("index-go");
    let output = impact(&root, &index, &fixture.0, "Load")?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let commands = commands(&output)?;
    assert_eq!(
        commands.len(),
        2,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    for command in commands {
        let output = execute(&root, &fixture.0, &command)?;
        assert!(
            output.status.success(),
            "{command}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[test]
fn nested_cargo_test_commands_use_manifests_and_run_multiple_tests() -> TestResult {
    let fixture = TempDir::new("impact-cargo-commands")?;
    let root = fixture.0.join("query root");
    let package = "rust crate's $workspace";
    fixture.write(
        &root,
        &format!("{package}/Cargo.toml"),
        "[package]\nname = \"storage-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    fixture.write(
        &root,
        &format!("{package}/src/lib.rs"),
        "pub fn open_storage() -> usize { 1 }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn reads_storage() { assert_eq!(super::open_storage(), 1); }\n    #[test]\n    fn writes_storage() { assert_eq!(super::open_storage(), 1); }\n}\n",
    )?;

    let index = fixture.0.join("index-rust");
    let output = impact(&root, &index, &fixture.0, "open_storage")?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let commands = commands(&output)?;
    assert_eq!(
        commands.len(),
        1,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let output = execute(&root, &fixture.0, &commands[0])?;
    assert!(
        output.status.success(),
        "{}\n{}\n{}",
        commands[0],
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("2 passed"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

#[test]
fn python_test_commands_use_pytest_node_ids() -> TestResult {
    let fixture = TempDir::new("impact-python-commands")?;
    let root = fixture.0.join("query root");
    fixture.write(
        &root,
        "app/storage.py",
        "def read_record():\n    return 1\n",
    )?;
    fixture.write(
        &root,
        "tests/test_storage's_file.py",
        "from app.storage import read_record\n\ndef test_reads_record():\n    assert read_record() == 1\n\nclass TestStorage:\n    def test_reads_record_method(self):\n        assert read_record() == 1\n",
    )?;
    let python_index = fixture.0.join("index-python");
    let python = impact(&root, &python_index, &fixture.0, "read_record")?;
    assert_eq!(
        python.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&python.stderr)
    );
    let python_commands = commands(&python)?;
    let expected = "pytest 'tests/test_storage'\\''s_file.py::TestStorage::test_reads_record_method' 'tests/test_storage'\\''s_file.py::test_reads_record'";
    assert_eq!(python_commands, [expected]);
    let shell_syntax = support::command("sh", &fixture.0)
        .args(["-n", "-c", &python_commands[0]])
        .current_dir(&root)
        .output()?;
    assert!(
        shell_syntax.status.success(),
        "{}",
        String::from_utf8_lossy(&shell_syntax.stderr)
    );
    Ok(())
}

#[test]
fn unsupported_test_runners_emit_test_facts_without_fake_run_commands() -> TestResult {
    let fixture = TempDir::new("impact-unsupported-commands")?;
    let root = fixture.0.join("query root");
    for relative in [
        "CMakeLists.txt",
        "include/store/repository.hpp",
        "src/repository.cpp",
        "src/main.cpp",
        "tests/repository_test.cpp",
    ] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/resolve/cpp-app")
            .join(relative);
        fixture.write(&root, relative, &fs::read_to_string(source)?)?;
    }
    let cpp_index = fixture.0.join("index-cpp");
    let cpp = impact(
        &root,
        &cpp_index,
        &fixture.0,
        "src/repository::store::MemoryRepository::find",
    )?;
    assert_eq!(
        cpp.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&cpp.stdout),
        String::from_utf8_lossy(&cpp.stderr)
    );
    let cpp_text = String::from_utf8(cpp.stdout.clone())?;
    assert!(
        cpp_text.lines().any(|line| line.starts_with("t ")),
        "{cpp_text}"
    );
    assert!(commands(&cpp)?.is_empty(), "{cpp_text}");
    Ok(())
}

#[test]
fn nested_swift_packages_get_a_package_path_and_a_test_filter() -> TestResult {
    let fixture = TempDir::new("impact-swift-package")?;
    let root = fixture.0.join("root");
    fixture.write(&root, "pkg/Package.swift", "// swift-tools-version:5.9\n")?;
    fixture.write(
        &root,
        "pkg/Sources/Lib/Lib.swift",
        "public func target() -> Int { 1 }\n",
    )?;
    fixture.write(
        &root,
        "pkg/Tests/LibTests/LibTests.swift",
        "import XCTest\n@testable import Lib\nfinal class LibTests: XCTestCase {\n    func testTarget() { XCTAssertEqual(target(), 1) }\n}\n",
    )?;
    let output = impact(&root, &fixture.0.join("index"), &fixture.0, "target")?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let found = commands(&output)?;
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].starts_with("swift test --package-path pkg --filter "),
        "{found:?}"
    );
    assert!(found[0].contains("LibTests"), "{found:?}");
    Ok(())
}
