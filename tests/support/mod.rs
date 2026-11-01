use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// Starts a test subprocess inside its fixture, without inherited product or Git overrides.
pub fn command(program: impl AsRef<OsStr>, fixture: &Path) -> Command {
    let mut command = Command::new(program);
    for (name, _) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        if name_text.starts_with("GIT_")
            || name_text.starts_with("XCALYGRAPH_")
            || matches!(
                name_text.as_ref(),
                "BASH_ENV" | "ENV" | "CDPATH" | "TAR_OPTIONS"
            )
        {
            command.env_remove(name);
        }
    }
    command
        .env("HOME", fixture.join("home"))
        .env("TMPDIR", fixture)
        .env("XDG_CONFIG_HOME", fixture.join("config"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("CARGO_TARGET_DIR", fixture.join("cargo-target"));
    command
}
