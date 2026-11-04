// SPDX-License-Identifier: LicenseRef-REAL-1.0

use clapsmith::clap::builder::styling::{AnsiColor, Style};
use clapsmith::{Args, Parser, Subcommand, Theme, help};

/// A test tool
#[derive(Parser)]
#[command(name = "tool")]
struct Cli {
    /// talk more
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a thing
    Run(Run),
}

#[derive(Args)]
struct Run {
    /// what to run
    #[arg(short, long)]
    command: String,
    /// how to group it
    #[arg(short, long, default_value = "run")]
    directive: String,
    /// extra words
    words: Vec<String>,
}

fn help_for(args: &[&str]) -> clapsmith::clap::Error {
    let mut cmd = help::apply(
        <Cli as clapsmith::clap::CommandFactory>::command(),
        &Theme::default(),
    );
    let argv = std::iter::once("tool").chain(args.iter().copied());
    match cmd.try_get_matches_from_mut(argv) {
        Err(e) if !e.use_stderr() => e,
        other => panic!("expected help, got {other:?}"),
    }
}

fn styled(style: Style, text: &str) -> String {
    format!("{style}{text}{style:#}")
}

#[test]
fn flags_and_commands_get_different_colours() {
    let theme = Theme::default();
    assert_ne!(theme.flag, theme.command);
    let ansi = help_for(&["--help"]).render().ansi().to_string();
    assert!(
        ansi.contains(&styled(theme.flag, "-v, --verbose")),
        "{ansi}"
    );
    assert!(ansi.contains(&styled(theme.command, "run")), "{ansi}");
    assert!(ansi.contains(&styled(theme.title, "tool")), "{ansi}");
    assert!(ansi.contains(&styled(theme.heading, "Commands:")), "{ansi}");
}

#[test]
fn plain_text_has_no_escape_codes() {
    let text = help_for(&["--help"]).to_string();
    assert!(!text.contains('\x1b'));
    assert!(text.starts_with("tool\n  A test tool\n"), "{text}");
    assert!(text.contains("$ tool [options] <command>"), "{text}");
}

#[test]
fn subcommand_help_shows_path_globals_notes_and_positionals() {
    let text = help_for(&["run", "--help"]).to_string();
    assert!(text.starts_with("tool run\n  Run a thing\n"), "{text}");
    assert!(text.contains("$ tool run [options] [WORDS]..."), "{text}");
    assert!(text.contains("-c, --command <COMMAND>"), "{text}");
    assert!(text.contains("what to run [required]"), "{text}");
    assert!(text.contains("how to group it [default: run]"), "{text}");
    assert!(
        text.contains("-v, --verbose"),
        "global flag missing: {text}"
    );
}

#[test]
fn columns_line_up() {
    let text = help_for(&["run", "--help"]).to_string();
    let starts: Vec<usize> = ["what to run", "how to group it", "talk more"]
        .iter()
        .map(|d| {
            let line = text.lines().find(|l| l.contains(d)).unwrap();
            line.find(d).unwrap()
        })
        .collect();
    assert!(
        starts.windows(2).all(|w| w[0] == w[1]),
        "{starts:?}\n{text}"
    );
}

#[test]
fn plain_theme_has_no_styles() {
    let ansi = help::render(
        &<Cli as clapsmith::clap::CommandFactory>::command(),
        "tool",
        &Theme::plain(),
    )
    .ansi()
    .to_string();
    assert!(!ansi.contains('\x1b'), "{ansi}");
}

#[test]
fn default_palette_matches_the_cpp_godo() {
    let theme = Theme::default();
    assert_eq!(theme.flag.get_fg_color(), Some(AnsiColor::Magenta.into()));
    assert_eq!(theme.command.get_fg_color(), Some(AnsiColor::Green.into()));
    assert_eq!(theme.heading.get_fg_color(), Some(AnsiColor::Cyan.into()));
    assert_eq!(theme.title.get_fg_color(), Some(AnsiColor::Blue.into()));
}

#[test]
fn after_help_keeps_its_colours() {
    use clapsmith::clap::builder::StyledStr;
    use std::fmt::Write;
    let theme = Theme::default();
    let mut extra = StyledStr::new();
    let _ = write!(extra, "{}Extra:{:#}", theme.heading, theme.heading);
    let cmd = <Cli as clapsmith::clap::CommandFactory>::command().after_help(extra);
    let ansi = help::render(&cmd, "tool", &theme).ansi().to_string();
    assert!(ansi.contains(&styled(theme.heading, "Extra:")), "{ansi}");
}
