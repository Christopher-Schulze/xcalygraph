//! Command-line interface: argument parsing, help text and exit statuses (`docs/spec.md` section 2).

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::domain::facts::SymKind;
use crate::domain::filetype::FileType;

/// Every command, in help order.
const COMMANDS: [CommandName; 12] = [
    CommandName::Map,
    CommandName::Find,
    CommandName::Ctx,
    CommandName::Show,
    CommandName::Impact,
    CommandName::Path,
    CommandName::Audit,
    CommandName::Index,
    CommandName::Install,
    CommandName::Update,
    CommandName::Version,
    CommandName::Help,
];

/// Flags of every query command.
const QUERY_FLAGS: [Flag; 10] = [
    Flag::Root,
    Flag::Budget,
    Flag::Json,
    Flag::ReadOnly,
    Flag::IndexDir,
    Flag::Path,
    Flag::Lang,
    Flag::Kind,
    Flag::Exact,
    Flag::NoTests,
];

const SHOW_FLAGS: [Flag; 12] = [
    Flag::Root,
    Flag::Budget,
    Flag::Json,
    Flag::ReadOnly,
    Flag::IndexDir,
    Flag::Path,
    Flag::Lang,
    Flag::Kind,
    Flag::Depth,
    Flag::Exact,
    Flag::NoTests,
    Flag::All,
];

const IMPACT_FLAGS: [Flag; 12] = [
    Flag::Root,
    Flag::Budget,
    Flag::Json,
    Flag::ReadOnly,
    Flag::IndexDir,
    Flag::Path,
    Flag::Lang,
    Flag::Kind,
    Flag::Depth,
    Flag::Exact,
    Flag::NoTests,
    Flag::Diff,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandName {
    Map,
    Find,
    Ctx,
    Show,
    Impact,
    Path,
    Audit,
    Index,
    Install,
    Update,
    Version,
    Help,
}

impl CommandName {
    fn from_name(name: &str) -> Option<Self> {
        COMMANDS.into_iter().find(|command| command.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Find => "find",
            Self::Ctx => "ctx",
            Self::Show => "show",
            Self::Impact => "impact",
            Self::Path => "path",
            Self::Audit => "audit",
            Self::Index => "index",
            Self::Install => "install",
            Self::Update => "update",
            Self::Version => "version",
            Self::Help => "help",
        }
    }

    fn synopsis(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Find => "find TERMS...",
            Self::Ctx => "ctx TERMS...",
            Self::Show => "show NAME [--all]",
            Self::Impact => "impact NAME... | --diff [REV]",
            Self::Path => "path FROM TO",
            Self::Audit => "audit",
            Self::Index => "index [--drop]",
            Self::Install => "install [--no-link] [--skill-only]",
            Self::Update => "update [--check]",
            Self::Version => "version",
            Self::Help => "help [COMMAND]",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::Map => {
                "orientation: languages, modules, entrypoints, hubs, cycles, layers, deps, env"
            }
            Self::Find => "ranked symbols and files, one line each",
            Self::Ctx => "context bundle: best matches plus related symbols with signatures",
            Self::Show => "card of a symbol or file: members, types, callers, callees, tests, env",
            Self::Impact => "changed symbols, affected dependents by depth, tests and run commands",
            Self::Path => "shortest dependency path between two names",
            Self::Audit => {
                "dead code, unused exports, cycles, stale doc references, env drift, hidden coupling, hotspots"
            }
            Self::Index => "build or refresh the index explicitly; --drop deletes it",
            Self::Install => {
                "install the binary to ~/.local/bin and the skill to ~/.agents/skills/xcalygraph"
            }
            Self::Update => "self-update from GitHub Releases and refresh a stale skill",
            Self::Version => {
                "binary, index format and installed skill versions; also -V, --version"
            }
            Self::Help => "syntax reference; also -h, --help, <command> --help",
        }
    }

    /// Accepted positional arguments: minimum and maximum count.
    fn arity(self) -> (usize, usize) {
        match self {
            Self::Find | Self::Ctx => (1, usize::MAX),
            Self::Show => (1, 1),
            Self::Impact => (0, usize::MAX),
            Self::Path => (2, 2),
            Self::Map
            | Self::Audit
            | Self::Index
            | Self::Install
            | Self::Update
            | Self::Version => (0, 0),
            Self::Help => (0, 1),
        }
    }

    fn flags(self) -> &'static [Flag] {
        match self {
            Self::Map | Self::Find | Self::Ctx | Self::Path | Self::Audit => &QUERY_FLAGS,
            Self::Show => &SHOW_FLAGS,
            Self::Impact => &IMPACT_FLAGS,
            Self::Index => &[Flag::Root, Flag::IndexDir, Flag::Drop],
            Self::Install => &[Flag::NoLink, Flag::SkillOnly],
            Self::Update => &[Flag::Check],
            Self::Version | Self::Help => &[],
        }
    }
}

/// Every flag, in help order.
const FLAGS: [Flag; 17] = [
    Flag::Root,
    Flag::Budget,
    Flag::Json,
    Flag::ReadOnly,
    Flag::IndexDir,
    Flag::Path,
    Flag::Lang,
    Flag::Kind,
    Flag::Depth,
    Flag::Exact,
    Flag::NoTests,
    Flag::All,
    Flag::Diff,
    Flag::Drop,
    Flag::NoLink,
    Flag::SkillOnly,
    Flag::Check,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Root,
    Budget,
    Json,
    ReadOnly,
    IndexDir,
    Path,
    Lang,
    Kind,
    Depth,
    Exact,
    NoTests,
    All,
    Diff,
    Drop,
    NoLink,
    SkillOnly,
    Check,
}

impl Flag {
    fn long(self) -> &'static str {
        match self {
            Self::Root => "--root",
            Self::Budget => "--budget",
            Self::Json => "--json",
            Self::ReadOnly => "--read-only",
            Self::IndexDir => "--index-dir",
            Self::Path => "--path",
            Self::Lang => "--lang",
            Self::Kind => "--kind",
            Self::Depth => "--depth",
            Self::Exact => "--exact",
            Self::NoTests => "--no-tests",
            Self::All => "--all",
            Self::Diff => "--diff",
            Self::Drop => "--drop",
            Self::NoLink => "--no-link",
            Self::SkillOnly => "--skill-only",
            Self::Check => "--check",
        }
    }

    fn short(self) -> Option<&'static str> {
        match self {
            Self::Root => Some("-C"),
            Self::Budget => Some("-b"),
            Self::Path => Some("-p"),
            Self::Lang => Some("-l"),
            Self::Kind => Some("-k"),
            Self::Depth => Some("-d"),
            _ => None,
        }
    }

    fn value_name(self) -> Option<&'static str> {
        match self {
            Self::Root | Self::IndexDir => Some("PATH"),
            Self::Budget | Self::Depth => Some("N"),
            Self::Path => Some("GLOB"),
            Self::Lang | Self::Kind => Some("LIST"),
            _ => None,
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::Root => "index root; default: nearest ancestor with .git, else cwd",
            Self::Budget => "output budget in tokens (bytes / 3)",
            Self::Json => "JSON Lines output",
            Self::ReadOnly => "never build or refresh; exit 5 if missing or stale",
            Self::IndexDir => "index base; default $XCALYGRAPH_INDEX_DIR, else ~/.cache/xcalygraph",
            Self::Path => "keep results under GLOB (gitignore syntax); repeatable",
            Self::Lang => "comma-separated language ids",
            Self::Kind => "comma-separated symbol kinds",
            Self::Depth => "traversal depth (show 1, impact 3)",
            Self::Exact => "drop inferred facts",
            Self::NoTests => "drop test, generated and fixture files",
            Self::All => "lift the per-section caps of show",
            Self::Diff => "impact of git changes against REV (default HEAD, work tree included)",
            Self::Drop => "delete the index of the root",
            Self::NoLink => "skip the Claude Code link (made when ~/.claude exists)",
            Self::SkillOnly => "install the skill only, not the binary",
            Self::Check => "report an available update without installing it",
        }
    }

    fn label(self) -> String {
        let names = match self.short() {
            Some(short) => format!("{short}, {}", self.long()),
            None => self.long().to_owned(),
        };
        match self.value_name() {
            Some(value) => format!("{names} {value}"),
            None => names,
        }
    }

    fn from_arg(arg: &str) -> Option<Self> {
        FLAGS
            .into_iter()
            .find(|flag| flag.long() == arg || flag.short() == Some(arg))
    }
}

/// Flag values shared by all commands; absent flags keep their defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    pub root: Option<PathBuf>,
    pub budget: Option<usize>,
    pub json: bool,
    pub read_only: bool,
    pub index_dir: Option<PathBuf>,
    pub paths: Vec<String>,
    pub langs: Vec<FileType>,
    pub kinds: Vec<SymKind>,
    pub depth: Option<usize>,
    pub exact: bool,
    pub no_tests: bool,
    pub all: bool,
    pub diff: bool,
    pub drop: bool,
    pub no_link: bool,
    pub skill_only: bool,
    pub check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub command: CommandName,
    pub args: Vec<String>,
    pub options: Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Overview without a topic, else the entry of one command.
    Help(Option<CommandName>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum UsageError {
    MissingCommand,
    UnknownCommand(String),
    UnexpectedArgument(String),
    MissingArgument(&'static str),
    UnknownFlag(String),
    FlagNotAccepted(&'static str, &'static str),
    MissingValue(&'static str),
    InvalidValue(&'static str, String),
}

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCommand => formatter.write_str("missing command"),
            Self::UnknownCommand(name) => write!(formatter, "unknown command '{name}'"),
            Self::UnexpectedArgument(arg) => write!(formatter, "unexpected argument '{arg}'"),
            Self::MissingArgument(command) => write!(formatter, "'{command}' needs an argument"),
            Self::UnknownFlag(flag) => write!(formatter, "unknown flag '{flag}'"),
            Self::FlagNotAccepted(flag, command) => {
                write!(formatter, "'{command}' does not accept {flag}")
            }
            Self::MissingValue(flag) => write!(formatter, "{flag} needs a value"),
            Self::InvalidValue(flag, value) => write!(formatter, "invalid {flag} value '{value}'"),
        }
    }
}

/// Process exit statuses (`docs/spec.md` section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Answer,
    Runtime,
    Usage,
    Ambiguous,
    NoMatch,
    Stale,
}

impl From<ExitStatus> for ExitCode {
    fn from(status: ExitStatus) -> Self {
        match status {
            ExitStatus::Answer => Self::SUCCESS,
            ExitStatus::Runtime => Self::from(1),
            ExitStatus::Usage => Self::from(2),
            ExitStatus::Ambiguous => Self::from(3),
            ExitStatus::NoMatch => Self::from(4),
            ExitStatus::Stale => Self::from(5),
        }
    }
}

/// Parses the arguments after the program name.
pub fn parse<I>(args: I) -> Result<Invocation, UsageError>
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<String> = args
        .into_iter()
        .enumerate()
        .map(|(index, arg)| {
            arg.into_string().map_err(|raw| match index {
                0 => UsageError::UnknownCommand(lossy(&raw)),
                _ => UsageError::UnexpectedArgument(lossy(&raw)),
            })
        })
        .collect::<Result<_, _>>()?;
    let (first, rest) = args.split_first().ok_or(UsageError::MissingCommand)?;
    let command = parse_command_name(first)?;
    if let [flag] = rest
        && is_help_flag(flag)
    {
        return Ok(help_for(Some(command)));
    }
    let mut invocation = Invocation {
        command,
        args: Vec::new(),
        options: Options::default(),
    };
    let mut rest = rest.iter();
    let mut positional_only = false;
    while let Some(arg) = rest.next() {
        if positional_only || !arg.starts_with('-') || arg == "-" {
            invocation.args.push(arg.clone());
            continue;
        }
        if arg == "--" {
            positional_only = true;
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let flag = Flag::from_arg(name).ok_or_else(|| {
            if command == CommandName::Help {
                UsageError::UnknownCommand(arg.clone())
            } else {
                UsageError::UnknownFlag(arg.clone())
            }
        })?;
        if !command.flags().contains(&flag) {
            return Err(UsageError::FlagNotAccepted(flag.long(), command.name()));
        }
        let value = match (flag.value_name(), inline) {
            (None, None) => None,
            (None, Some(_)) => return Err(UsageError::InvalidValue(flag.long(), arg.clone())),
            (Some(_), Some(value)) => Some(value),
            (Some(_), None) => Some(
                rest.next()
                    .cloned()
                    .ok_or(UsageError::MissingValue(flag.long()))?,
            ),
        };
        apply(&mut invocation.options, flag, value)?;
    }
    let (min, max) = command.arity();
    if invocation.args.len() < min {
        return Err(UsageError::MissingArgument(command.name()));
    }
    if let Some(extra) = invocation.args.get(max) {
        return Err(UsageError::UnexpectedArgument(extra.clone()));
    }
    if command == CommandName::Help
        && let Some(topic) = invocation.args.first()
    {
        parse_topic(topic)?;
    }
    Ok(invocation)
}

fn apply(options: &mut Options, flag: Flag, value: Option<String>) -> Result<(), UsageError> {
    let value = value.unwrap_or_default();
    let invalid = || UsageError::InvalidValue(flag.long(), value.clone());
    match flag {
        Flag::Root => options.root = Some(PathBuf::from(&value)),
        Flag::IndexDir => options.index_dir = Some(PathBuf::from(&value)),
        Flag::Budget => {
            options.budget = Some(
                value
                    .parse()
                    .ok()
                    .filter(|&n: &usize| n > 0)
                    .ok_or_else(invalid)?,
            );
        }
        Flag::Depth => options.depth = Some(value.parse().map_err(|_| invalid())?),
        Flag::Json => options.json = true,
        Flag::ReadOnly => options.read_only = true,
        Flag::Exact => options.exact = true,
        Flag::NoTests => options.no_tests = true,
        Flag::Drop => options.drop = true,
        Flag::All => options.all = true,
        Flag::Diff => options.diff = true,
        Flag::Check => options.check = true,
        Flag::NoLink => options.no_link = true,
        Flag::SkillOnly => options.skill_only = true,
        Flag::Path => options.paths.push(value),
        Flag::Lang => {
            for id in value.split(',').filter(|id| !id.is_empty()) {
                options
                    .langs
                    .push(FileType::from_id(id).ok_or_else(invalid)?);
            }
        }
        Flag::Kind => {
            for id in value.split(',').filter(|id| !id.is_empty()) {
                options
                    .kinds
                    .push(SymKind::from_id(id).ok_or_else(invalid)?);
            }
        }
    }
    Ok(())
}

fn help_for(topic: Option<CommandName>) -> Invocation {
    Invocation {
        command: CommandName::Help,
        args: topic
            .map(|name| vec![name.name().to_owned()])
            .unwrap_or_default(),
        options: Options::default(),
    }
}

/// The command that needs no index, if the invocation is one.
pub fn local_command(invocation: &Invocation) -> Option<Command> {
    match invocation.command {
        CommandName::Help => Some(Command::Help(
            invocation
                .args
                .first()
                .and_then(|topic| CommandName::from_name(topic)),
        )),
        _ => None,
    }
}

/// Output of commands that need no index.
pub fn render(command: Command) -> String {
    match command {
        Command::Help(None) => overview(),
        Command::Help(Some(name)) => {
            let mut text = format!("xcalygraph {}\n  {}\n", name.synopsis(), name.summary());
            for flag in name.flags() {
                text.push_str(&format!("  {}  {}\n", flag.label(), flag.summary()));
            }
            text
        }
    }
}

fn overview() -> String {
    let width = COMMANDS
        .iter()
        .map(|command| command.synopsis().len())
        .max()
        .unwrap_or(0);
    let mut text = String::from("xcalygraph <command> [args] [flags]\ncommands:\n");
    for command in COMMANDS {
        text.push_str(&format!(
            "  {:width$}  {}\n",
            command.synopsis(),
            command.summary()
        ));
    }
    let used: Vec<Flag> = FLAGS
        .into_iter()
        .filter(|flag| {
            COMMANDS
                .iter()
                .any(|command| command.flags().contains(flag))
        })
        .collect();
    if !used.is_empty() {
        let width = used
            .iter()
            .map(|flag| flag.label().len())
            .max()
            .unwrap_or(0);
        text.push_str("flags:\n");
        for flag in used {
            text.push_str(&format!("  {:width$}  {}\n", flag.label(), flag.summary()));
        }
    }
    text.push_str(
        "exit codes: 0 answer, 1 runtime error, 2 usage error, 3 ambiguous name, 4 no match, \
         5 index missing or stale under --read-only\n",
    );
    text
}

fn parse_command_name(arg: &str) -> Result<CommandName, UsageError> {
    match arg {
        "-h" | "--help" => Ok(CommandName::Help),
        "-V" | "--version" => Ok(CommandName::Version),
        _ => parse_topic(arg),
    }
}

fn parse_topic(arg: &str) -> Result<CommandName, UsageError> {
    CommandName::from_name(arg).ok_or_else(|| UsageError::UnknownCommand(arg.to_owned()))
}

fn is_help_flag(arg: &str) -> bool {
    matches!(arg, "-h" | "--help")
}

fn lossy(arg: &OsStr) -> String {
    arg.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Invocation, UsageError> {
        parse(args.iter().map(OsString::from))
    }

    type Parsed = Result<(CommandName, Vec<String>), UsageError>;

    fn command(args: &[&str]) -> Parsed {
        parse_strs(args).map(|invocation| (invocation.command, invocation.args))
    }

    fn ok(name: CommandName, args: &[&str]) -> Parsed {
        Ok((name, args.iter().map(|arg| (*arg).to_owned()).collect()))
    }

    #[test]
    fn parse_resolves_commands_aliases_and_errors() {
        use CommandName::{Help, Index, Version};
        let unknown = |name: &str| Err(UsageError::UnknownCommand(name.to_owned()));
        let unexpected = |arg: &str| Err(UsageError::UnexpectedArgument(arg.to_owned()));
        let cases: [(&[&str], Parsed); 19] = [
            (&[], Err(UsageError::MissingCommand)),
            (&["help"], ok(Help, &[])),
            (&["-h"], ok(Help, &[])),
            (&["--help"], ok(Help, &[])),
            (&["help", "version"], ok(Help, &["version"])),
            (&["help", "help"], ok(Help, &["help"])),
            (&["help", "--help"], ok(Help, &["help"])),
            (&["version"], ok(Version, &[])),
            (&["-V"], ok(Version, &[])),
            (&["--version"], ok(Version, &[])),
            (&["version", "--help"], ok(Help, &["version"])),
            (&["index", "-h"], ok(Help, &["index"])),
            (&["index"], ok(Index, &[])),
            (&["nope"], unknown("nope")),
            (&["help", "nope"], unknown("nope")),
            (&["help", "-V"], unknown("-V")),
            (&["version", "extra"], unexpected("extra")),
            (&["help", "version", "extra"], unexpected("extra")),
            (
                &["version", "--help", "extra"],
                Err(UsageError::UnknownFlag("--help".to_owned())),
            ),
        ];
        for (args, want) in cases {
            assert_eq!(command(args), want, "args {args:?}");
        }
    }

    #[test]
    fn flags_take_values_in_both_forms_and_are_checked_per_command() {
        let parsed = parse_strs(&["index", "-C", "/r", "--index-dir=/i", "--drop"]);
        let options = parsed.map(|invocation| invocation.options);
        assert_eq!(
            options,
            Ok(Options {
                root: Some(PathBuf::from("/r")),
                index_dir: Some(PathBuf::from("/i")),
                drop: true,
                ..Options::default()
            })
        );
        assert_eq!(
            parse_strs(&["index", "--json"]),
            Err(UsageError::FlagNotAccepted("--json", "index"))
        );
        assert_eq!(
            parse_strs(&["index", "-C"]),
            Err(UsageError::MissingValue("--root"))
        );
        assert_eq!(
            parse_strs(&["index", "--drop=1"]),
            Err(UsageError::InvalidValue("--drop", "--drop=1".to_owned()))
        );
        assert_eq!(
            parse_strs(&["index", "--bogus"]),
            Err(UsageError::UnknownFlag("--bogus".to_owned()))
        );
    }

    #[test]
    fn values_are_validated() {
        let mut options = Options::default();
        assert_eq!(
            apply(&mut options, Flag::Lang, Some("go,rust".to_owned())),
            Ok(())
        );
        assert_eq!(options.langs, [FileType::Go, FileType::Rust]);
        assert_eq!(
            apply(&mut options, Flag::Kind, Some("fn,struct".to_owned())),
            Ok(())
        );
        assert_eq!(options.kinds, [SymKind::Fn, SymKind::Struct]);
        assert_eq!(
            apply(&mut options, Flag::Path, Some("src/**".to_owned())),
            Ok(())
        );
        assert_eq!(
            apply(&mut options, Flag::Depth, Some("2".to_owned())),
            Ok(())
        );
        assert_eq!(options.depth, Some(2));
        let invalid =
            |flag: &'static str, value: &str| Err(UsageError::InvalidValue(flag, value.to_owned()));
        assert_eq!(
            apply(&mut options, Flag::Lang, Some("klingon".to_owned())),
            invalid("--lang", "klingon")
        );
        assert_eq!(
            apply(&mut options, Flag::Kind, Some("thing".to_owned())),
            invalid("--kind", "thing")
        );
        assert_eq!(
            apply(&mut options, Flag::Budget, Some("0".to_owned())),
            invalid("--budget", "0")
        );
        assert_eq!(
            apply(&mut options, Flag::Depth, Some("-1".to_owned())),
            invalid("--depth", "-1")
        );
    }

    #[cfg(unix)]
    #[test]
    fn parse_reports_non_utf8_arguments_lossily() {
        use std::os::unix::ffi::OsStringExt;
        let arg = OsString::from_vec(vec![b'x', 0xff]);
        assert_eq!(
            parse([arg.clone()]),
            Err(UsageError::UnknownCommand("x\u{fffd}".to_owned()))
        );
        assert_eq!(
            parse([OsString::from("index"), arg]),
            Err(UsageError::UnexpectedArgument("x\u{fffd}".to_owned()))
        );
    }

    #[test]
    fn overview_lists_commands_flags_and_exit_codes() {
        let overview = render(Command::Help(None));
        let lines: Vec<&str> = overview.lines().collect();
        assert_eq!(lines[0], "xcalygraph <command> [args] [flags]");
        assert_eq!(lines[1], "commands:");
        let flags_at = lines.iter().position(|line| *line == "flags:");
        assert_eq!(flags_at, Some(2 + COMMANDS.len()));
        for (offset, command) in COMMANDS.iter().enumerate() {
            let line = lines[2 + offset];
            assert!(
                line.starts_with(&format!("  {}", command.synopsis())),
                "{line}"
            );
            assert!(line.ends_with(command.summary()), "{line}");
        }
        let used = FLAGS
            .iter()
            .filter(|flag| {
                COMMANDS
                    .iter()
                    .any(|command| command.flags().contains(flag))
            })
            .count();
        assert_eq!(lines.len(), 2 + COMMANDS.len() + 1 + used + 1);
        for flag in FLAGS {
            assert!(overview.contains(&flag.label()), "{}", flag.long());
        }
        assert_eq!(
            lines[lines.len() - 1],
            "exit codes: 0 answer, 1 runtime error, 2 usage error, 3 ambiguous name, 4 no match, \
             5 index missing or stale under --read-only"
        );
    }

    #[test]
    fn command_help_shows_synopsis_summary_and_flags() {
        assert_eq!(
            render(Command::Help(Some(CommandName::Index))),
            "xcalygraph index [--drop]\n  build or refresh the index explicitly; --drop deletes it\n  \
             -C, --root PATH  index root; default: nearest ancestor with .git, else cwd\n  \
             --index-dir PATH  index base; default $XCALYGRAPH_INDEX_DIR, else ~/.cache/xcalygraph\n  \
             --drop  delete the index of the root\n"
        );
    }

    #[test]
    fn usage_errors_name_the_offending_argument() {
        let cases = [
            (UsageError::MissingCommand, "missing command"),
            (
                UsageError::UnknownCommand("x".to_owned()),
                "unknown command 'x'",
            ),
            (
                UsageError::UnexpectedArgument("y".to_owned()),
                "unexpected argument 'y'",
            ),
            (
                UsageError::MissingArgument("find"),
                "'find' needs an argument",
            ),
            (
                UsageError::UnknownFlag("--z".to_owned()),
                "unknown flag '--z'",
            ),
            (
                UsageError::FlagNotAccepted("--json", "index"),
                "'index' does not accept --json",
            ),
            (UsageError::MissingValue("--root"), "--root needs a value"),
            (
                UsageError::InvalidValue("--lang", "q".to_owned()),
                "invalid --lang value 'q'",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
    }
}
