//! Command-line interface: argument parsing, help text and exit statuses.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every command, in help order.
const COMMANDS: [CommandName; 2] = [CommandName::Help, CommandName::Version];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandName {
    Help,
    Version,
}

impl CommandName {
    fn from_name(name: &str) -> Option<Self> {
        COMMANDS.into_iter().find(|command| command.name() == name)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Version => "version",
        }
    }

    fn synopsis(self) -> &'static str {
        match self {
            Self::Help => "help [COMMAND]",
            Self::Version => "version",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::Help => "syntax reference; also -h, --help, <command> --help",
            Self::Version => "binary version; also -V, --version",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Overview without a topic, else the entry of one command.
    Help(Option<CommandName>),
    Version,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UsageError {
    MissingCommand,
    UnknownCommand(String),
    UnexpectedArgument(String),
}

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCommand => formatter.write_str("missing command"),
            Self::UnknownCommand(name) => write!(formatter, "unknown command '{name}'"),
            Self::UnexpectedArgument(arg) => write!(formatter, "unexpected argument '{arg}'"),
        }
    }
}

/// Process exit statuses (`docs/spec.md` section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Answer,
    Runtime,
    Usage,
}

impl From<ExitStatus> for ExitCode {
    fn from(status: ExitStatus) -> Self {
        match status {
            ExitStatus::Answer => Self::SUCCESS,
            ExitStatus::Runtime => Self::from(1),
            ExitStatus::Usage => Self::from(2),
        }
    }
}

/// Parses the arguments after the program name.
pub fn parse<I>(args: I) -> Result<Command, UsageError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let first = args.next().ok_or(UsageError::MissingCommand)?;
    let name = parse_command_name(&first)?;
    let rest: Vec<OsString> = args.collect();
    if let [flag] = rest.as_slice()
        && is_help_flag(flag)
    {
        return Ok(Command::Help(Some(name)));
    }
    let mut rest = rest.into_iter();
    let command = match name {
        CommandName::Help => Command::Help(rest.next().map(|arg| parse_topic(&arg)).transpose()?),
        CommandName::Version => Command::Version,
    };
    match rest.next() {
        Some(extra) => Err(UsageError::UnexpectedArgument(lossy(&extra))),
        None => Ok(command),
    }
}

/// Output of commands that need no index.
pub fn render(command: Command) -> String {
    match command {
        Command::Help(None) => overview(),
        Command::Help(Some(name)) => {
            format!("xcalygraph {}\n  {}\n", name.synopsis(), name.summary())
        }
        Command::Version => format!("xcalygraph {VERSION}\n"),
    }
}

fn overview() -> String {
    let width = COMMANDS
        .iter()
        .map(|command| command.synopsis().len())
        .max()
        .unwrap_or(0);
    let mut text = String::from("xcalygraph <command> [args]\ncommands:\n");
    for command in COMMANDS {
        text.push_str(&format!(
            "  {:width$}  {}\n",
            command.synopsis(),
            command.summary()
        ));
    }
    text.push_str("exit codes: 0 answer, 1 runtime error, 2 usage error\n");
    text
}

fn parse_command_name(arg: &OsStr) -> Result<CommandName, UsageError> {
    match arg.to_str() {
        Some("-h" | "--help") => Ok(CommandName::Help),
        Some("-V" | "--version") => Ok(CommandName::Version),
        _ => parse_topic(arg),
    }
}

fn parse_topic(arg: &OsStr) -> Result<CommandName, UsageError> {
    arg.to_str()
        .and_then(CommandName::from_name)
        .ok_or_else(|| UsageError::UnknownCommand(lossy(arg)))
}

fn is_help_flag(arg: &OsStr) -> bool {
    matches!(arg.to_str(), Some("-h" | "--help"))
}

fn lossy(arg: &OsStr) -> String {
    arg.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Command, UsageError> {
        parse(args.iter().map(OsString::from))
    }

    fn unknown(name: &str) -> Result<Command, UsageError> {
        Err(UsageError::UnknownCommand(name.to_owned()))
    }

    fn unexpected(arg: &str) -> Result<Command, UsageError> {
        Err(UsageError::UnexpectedArgument(arg.to_owned()))
    }

    #[test]
    fn parse_resolves_commands_aliases_and_errors() {
        let cases: [(&[&str], Result<Command, UsageError>); 18] = [
            (&[], Err(UsageError::MissingCommand)),
            (&["help"], Ok(Command::Help(None))),
            (&["-h"], Ok(Command::Help(None))),
            (&["--help"], Ok(Command::Help(None))),
            (
                &["help", "version"],
                Ok(Command::Help(Some(CommandName::Version))),
            ),
            (
                &["help", "help"],
                Ok(Command::Help(Some(CommandName::Help))),
            ),
            (
                &["help", "--help"],
                Ok(Command::Help(Some(CommandName::Help))),
            ),
            (&["version"], Ok(Command::Version)),
            (&["-V"], Ok(Command::Version)),
            (&["--version"], Ok(Command::Version)),
            (
                &["version", "--help"],
                Ok(Command::Help(Some(CommandName::Version))),
            ),
            (
                &["version", "-h"],
                Ok(Command::Help(Some(CommandName::Version))),
            ),
            (&["find"], unknown("find")),
            (&["help", "find"], unknown("find")),
            (&["help", "-V"], unknown("-V")),
            (&["version", "extra"], unexpected("extra")),
            (&["help", "version", "extra"], unexpected("extra")),
            (&["version", "--help", "extra"], unexpected("--help")),
        ];
        for (args, want) in cases {
            assert_eq!(parse_strs(args), want, "args {args:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn parse_reports_non_utf8_command_lossily() {
        use std::os::unix::ffi::OsStringExt;
        let arg = OsString::from_vec(vec![b'x', 0xff]);
        assert_eq!(parse([arg]), unknown("x\u{fffd}"));
    }

    #[test]
    fn overview_lists_every_command_aligned() {
        assert_eq!(
            render(Command::Help(None)),
            "xcalygraph <command> [args]\n\
             commands:\n  \
             help [COMMAND]  syntax reference; also -h, --help, <command> --help\n  \
             version         binary version; also -V, --version\n\
             exit codes: 0 answer, 1 runtime error, 2 usage error\n"
        );
    }

    #[test]
    fn command_help_shows_synopsis_and_summary() {
        assert_eq!(
            render(Command::Help(Some(CommandName::Help))),
            "xcalygraph help [COMMAND]\n  syntax reference; also -h, --help, <command> --help\n"
        );
    }

    #[test]
    fn usage_errors_name_the_offending_argument() {
        assert_eq!(UsageError::MissingCommand.to_string(), "missing command");
        assert_eq!(
            UsageError::UnknownCommand("x".to_owned()).to_string(),
            "unknown command 'x'"
        );
        assert_eq!(
            UsageError::UnexpectedArgument("y".to_owned()).to_string(),
            "unexpected argument 'y'"
        );
    }
}
