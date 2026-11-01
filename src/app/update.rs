//! `update`: self-update from GitHub Releases with checksum verification and atomic replace
//! (`docs/spec.md` section 10).

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

use super::error::AppError;
use super::install::VERSION;
use super::progress::Progress;

const LATEST: &str = "https://api.github.com/repos/Christopher-Schulze/xcalygraph/releases/latest";
/// Largest accepted download: release archives with every grammar stay far below.
const DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// Asset file name and download URL.
    pub assets: Vec<(String, String)>,
}

/// Release metadata URL; `XCALYGRAPH_UPDATE_URL` overrides it (also `file://` paths).
fn latest_url() -> String {
    std::env::var("XCALYGRAPH_UPDATE_URL").unwrap_or_else(|_| LATEST.to_owned())
}

/// Reads a URL (or `file://` path) in chunks, reporting the bytes to `progress` when given.
fn fetch(url: &str, progress: Option<&Progress>) -> Result<Vec<u8>, AppError> {
    let total: u64;
    let mut reader: Box<dyn Read> = if let Some(path) = url.strip_prefix("file://") {
        total = fs::metadata(path).map_or(0, |meta| meta.len());
        Box::new(fs::File::open(path).map_err(AppError::io(format!("read {path}")))?)
    } else {
        let response = ureq::get(url)
            .header(
                "User-Agent",
                concat!("xcalygraph/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|error| AppError::Invalid(format!("download {url}: {error}")))?;
        total = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        Box::new(
            response
                .into_body()
                .into_with_config()
                .limit(DOWNLOAD_LIMIT)
                .reader(),
        )
    };
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| AppError::Invalid(format!("download {url}: {error}")))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() as u64 > DOWNLOAD_LIMIT {
            return Err(AppError::Invalid(format!(
                "download {url}: over the size limit"
            )));
        }
        if let Some(progress) = progress {
            progress.bytes(bytes.len() as u64, total);
        }
    }
    Ok(bytes)
}

/// Reads `tag_name` and every `browser_download_url` of a GitHub release object.
pub fn parse_release(json: &str) -> Option<Release> {
    let tag = json_strings(json, "tag_name").into_iter().next()?;
    let assets = json_strings(json, "browser_download_url")
        .into_iter()
        .filter_map(|url| Some((url.rsplit('/').next()?.to_owned(), url)))
        .collect();
    Some(Release {
        version: tag.trim_start_matches('v').to_owned(),
        assets,
    })
}

/// Every string value of `key` in a JSON text (keys of nested objects included).
fn json_strings(json: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    let mut values = Vec::new();
    let mut rest = json;
    while let Some(found) = rest.find(&needle) {
        rest = &rest[found + needle.len()..];
        let Some(after_colon) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(body) = after_colon.trim_start().strip_prefix('"') else {
            continue;
        };
        let mut value = String::new();
        let mut chars = body.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some(other) => value.push(other),
                    None => break,
                },
                other => value.push(other),
            }
        }
        values.push(value);
    }
    values
}

/// `major.minor.patch` comparison; non-numeric parts compare as 0.
fn newer(candidate: &str, current: &str) -> bool {
    let parse = |version: &str| -> Vec<u64> {
        version
            .split(['.', '-', '+'])
            .take(3)
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    parse(candidate) > parse(current)
}

/// Asset name of this platform: `xcalygraph-<version>-<os>-<arch>.tar.gz`.
pub fn asset_name(version: &str) -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    };
    format!("xcalygraph-{version}-{os}-{arch}.tar.gz")
}

/// Checks the latest release; installs a newer one (download, checksum, smoke test, atomic
/// replace) and then the new binary's skill, or refreshes a stale skill of the current binary.
/// Each step is a progress phase; the result lines name what changed.
pub fn update(check: bool, progress: &mut Progress) -> Result<String, AppError> {
    progress.start("Release");
    let metadata = fetch(&latest_url(), None).inspect_err(|_| progress.fail("unreachable"))?;
    let release = parse_release(&String::from_utf8_lossy(&metadata)).ok_or_else(|| {
        progress.fail("no release metadata");
        AppError::Invalid("release metadata without tag_name".to_owned())
    })?;
    let available = newer(&release.version, VERSION);
    progress.pass(&if available {
        format!("{VERSION} -> {}", release.version)
    } else {
        format!("{VERSION} is the latest")
    });
    if check {
        return Ok(if available {
            format!(
                "update available: {VERSION} -> {}; run 'xcalygraph update'\n",
                release.version
            )
        } else {
            format!("xcalygraph {VERSION} is current\n")
        });
    }
    let home = super::install::home()?;
    if !available {
        progress.start("Skill");
        let line =
            super::install::install_skill(&home).inspect_err(|_| progress.fail("not written"))?;
        progress.pass(line.trim_end().trim_start_matches("skill "));
        return Ok(format!("xcalygraph {VERSION} is current\n{line}"));
    }
    let exe = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(AppError::io("locate the running binary"))?;
    replace_binary(&release, &exe, progress)?;
    progress.start("Skill");
    let installed = Command::new(&exe)
        .args(["install", "--skill-only"])
        .output()
        .map_err(|error| {
            progress.fail("new binary did not run");
            AppError::Io("run install with the new binary".to_owned(), error)
        })?;
    let lines = String::from_utf8_lossy(&installed.stdout).into_owned();
    if installed.status.success() {
        progress.pass(
            lines
                .lines()
                .next()
                .unwrap_or_default()
                .trim_start_matches("skill "),
        );
    } else {
        progress.fail("run 'xcalygraph install'");
    }
    Ok(format!(
        "updated {VERSION} -> {} at {}\n{lines}",
        release.version,
        exe.display()
    ))
}

/// Downloads the platform asset, verifies it against `SHA256SUMS`, extracts the binary,
/// smoke-tests it (`version` must report the release) and replaces `exe` atomically. The old
/// binary stays untouched on any failure.
pub fn replace_binary(
    release: &Release,
    exe: &Path,
    progress: &mut Progress,
) -> Result<(), AppError> {
    let wanted = asset_name(&release.version);
    let url_of = |name: &str| {
        release
            .assets
            .iter()
            .find(|(asset, _)| asset == name)
            .map(|(_, url)| url.clone())
            .ok_or_else(|| {
                AppError::Invalid(format!("release {} has no asset {name}", release.version))
            })
    };
    progress.start("Download");
    let archive =
        fetch(&url_of(&wanted)?, Some(progress)).inspect_err(|_| progress.fail(&wanted))?;
    progress.pass(&wanted);
    progress.start("Verify");
    let sums = String::from_utf8_lossy(
        &fetch(&url_of("SHA256SUMS")?, None).inspect_err(|_| progress.fail("no SHA256SUMS"))?,
    )
    .into_owned();
    let expected = sums
        .lines()
        .find_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == wanted).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| {
            progress.fail("not listed in SHA256SUMS");
            AppError::Invalid(format!("SHA256SUMS lists no {wanted}"))
        })?;
    let actual: String = Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != expected {
        progress.fail("checksum mismatch");
        return Err(AppError::Invalid(format!(
            "checksum mismatch for {wanted}: expected {expected}, got {actual}"
        )));
    }
    progress.pass(&format!("SHA-256 {}", &actual[..12]));
    let binary = extract_binary(&archive)?;
    let (temp, mut file) = crate::infra::file_access::create_beside(exe, "update", 0o755).map_err(
        AppError::io(format!("create a new binary beside {}", exe.display())),
    )?;
    let written = file.write_all(&binary);
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(AppError::Io(format!("write {}", temp.display()), error));
    }
    progress.start("Smoke test");
    if let Err(error) = smoke_test(&temp, &release.version) {
        let _ = fs::remove_file(&temp);
        progress.fail("old binary kept");
        return Err(error);
    }
    progress.pass(&format!("xcalygraph {} runs", release.version));
    progress.start("Publish");
    fs::rename(&temp, exe).map_err(|error| {
        let _ = fs::remove_file(&temp);
        progress.fail("old binary kept");
        AppError::Io(format!("replace {}", exe.display()), error)
    })?;
    progress.pass(&exe.display().to_string());
    Ok(())
}

/// Runs `candidate version`; its first line must name `version`.
fn smoke_test(candidate: &Path, version: &str) -> Result<(), AppError> {
    let output = Command::new(candidate)
        .arg("version")
        .output()
        .map_err(AppError::io("run the downloaded binary"))?;
    let first = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if output.status.success() && first == format!("xcalygraph {version}") {
        Ok(())
    } else {
        Err(AppError::Invalid(format!(
            "downloaded binary failed its smoke test (reported '{first}', expected 'xcalygraph {version}')"
        )))
    }
}

fn extract_binary(archive: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    let entries = tar
        .entries()
        .map_err(AppError::io("read release archive"))?;
    for entry in entries {
        let mut entry = entry.map_err(AppError::io("read release archive"))?;
        let is_binary = entry
            .path()
            .ok()
            .and_then(|path| path.file_name().map(|name| name == "xcalygraph"))
            .unwrap_or(false);
        if is_binary {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(AppError::io("extract the binary"))?;
            return Ok(bytes);
        }
    }
    Err(AppError::Invalid(
        "release archive contains no xcalygraph binary".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::testdir::TestDir;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    fn archive(content: &[u8]) -> std::io::Result<Vec<u8>> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, "xcalygraph-9.9.9/xcalygraph", content)?;
        builder.into_inner()?.finish()
    }

    fn release(dir: &TestDir, archive_bytes: &[u8], sum: &str) -> std::io::Result<Release> {
        let name = asset_name("9.9.9");
        let archive_path = dir.write(&name, "")?;
        fs::write(&archive_path, archive_bytes)?;
        let sums = dir.write("SHA256SUMS", &format!("{sum}  {name}\n"))?;
        Ok(Release {
            version: "9.9.9".to_owned(),
            assets: vec![
                (name, format!("file://{}", archive_path.display())),
                (
                    "SHA256SUMS".to_owned(),
                    format!("file://{}", sums.display()),
                ),
            ],
        })
    }

    #[test]
    fn verified_archive_replaces_the_binary() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("update")?;
        let exe = dir.write("bin/xcalygraph", "old")?;
        // The smoke test runs the candidate: a script that reports the release version.
        let script = b"#!/bin/sh\necho 'xcalygraph 9.9.9'\n";
        let bytes = archive(script)?;
        let sum: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        replace_binary(
            &release(&dir, &bytes, &sum)?,
            &exe,
            &mut Progress::new(false, false),
        )?;
        assert_eq!(fs::read(&exe)?, script);
        Ok(())
    }

    #[test]
    fn checksum_mismatch_keeps_the_old_binary() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("update-bad")?;
        let exe = dir.write("bin/xcalygraph", "old")?;
        let bytes = archive(b"tampered")?;
        let result = replace_binary(
            &release(&dir, &bytes, &"0".repeat(64))?,
            &exe,
            &mut Progress::new(false, false),
        );
        assert!(
            matches!(result, Err(AppError::Invalid(message)) if message.contains("checksum mismatch"))
        );
        assert_eq!(fs::read(&exe)?, b"old");
        Ok(())
    }

    #[test]
    fn failed_smoke_test_keeps_the_old_binary() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TestDir::new("update-smoke")?;
        let exe = dir.write("bin/xcalygraph", "old")?;
        let bytes = archive(b"#!/bin/sh\necho 'xcalygraph 0.0.0'\n")?;
        let sum: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let result = replace_binary(
            &release(&dir, &bytes, &sum)?,
            &exe,
            &mut Progress::new(false, false),
        );
        assert!(
            matches!(result, Err(AppError::Invalid(message)) if message.contains("smoke test"))
        );
        assert_eq!(fs::read(&exe)?, b"old");
        Ok(())
    }

    #[test]
    fn release_metadata_and_versions_parse() {
        let json = r#"{"tag_name": "v1.2.3", "assets": [{"name": "a", "browser_download_url": "https://x/dl/xcalygraph-1.2.3-darwin-arm64.tar.gz"}, {"browser_download_url": "https://x/dl/SHA256SUMS"}]}"#;
        let release = parse_release(json);
        assert_eq!(
            release,
            Some(Release {
                version: "1.2.3".to_owned(),
                assets: vec![
                    (
                        "xcalygraph-1.2.3-darwin-arm64.tar.gz".to_owned(),
                        "https://x/dl/xcalygraph-1.2.3-darwin-arm64.tar.gz".to_owned()
                    ),
                    (
                        "SHA256SUMS".to_owned(),
                        "https://x/dl/SHA256SUMS".to_owned()
                    ),
                ],
            })
        );
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
        assert!(parse_release("{}").is_none());
    }
}
