// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Help screens drawn by clapsmith instead of clap's default layout.
//!
//! clap's [`Styles`] has one style for every literal, so flags and command
//! names can't get different colours. clapsmith renders each command's help
//! itself from clap's command tree and installs it with
//! [`Command::override_help`]. clap still decides which command's help to
//! show and strips the colours when the output isn't a terminal.

use std::fmt::Write;

use clap::builder::StyledStr;
use clap::builder::styling::{AnsiColor, Effects, Style, Styles};
use clap::{Arg, Command};

/// The colours clapsmith uses for help and usage errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// The command's path at the top, and the program name in usage lines.
    pub title: Style,
    /// Section headings: `Usage:`, `Options:`, `Commands:`.
    pub heading: Style,
    /// Flag and option names: `-v, --verbose`.
    pub flag: Style,
    /// Subcommand names.
    pub command: Style,
    /// Value names: `<PATH>`.
    pub placeholder: Style,
    /// Notes after a description: `[default: run]`, `[required]`.
    pub note: Style,
    /// `error:` in usage errors.
    pub error: Style,
    /// The offending value in usage errors.
    pub invalid: Style,
}

impl Default for Theme {
    /// Blue title, cyan headings, magenta flags, green commands.
    fn default() -> Self {
        Self {
            title: AnsiColor::Blue.on_default() | Effects::BOLD,
            heading: AnsiColor::Cyan.on_default() | Effects::BOLD,
            flag: AnsiColor::Magenta.on_default(),
            command: AnsiColor::Green.on_default(),
            placeholder: AnsiColor::BrightBlack.on_default(),
            note: AnsiColor::BrightBlack.on_default(),
            error: AnsiColor::Red.on_default() | Effects::BOLD,
            invalid: AnsiColor::Yellow.on_default(),
        }
    }
}

impl Theme {
    /// No colours or effects at all.
    pub fn plain() -> Self {
        let none = Style::new();
        Self {
            title: none,
            heading: none,
            flag: none,
            command: none,
            placeholder: none,
            note: none,
            error: none,
            invalid: none,
        }
    }

    /// The same palette as clap [`Styles`], for usage errors.
    pub fn styles(&self) -> Styles {
        Styles::styled()
            .header(self.heading)
            .usage(self.heading)
            .literal(self.flag)
            .placeholder(self.placeholder)
            .error(self.error)
            .valid(self.command)
            .invalid(self.invalid)
    }
}

/// Applies `theme` to `cmd` and every subcommand: clapsmith's help screens
/// plus matching colours for clap's usage errors.
pub fn apply(cmd: Command, theme: &Theme) -> Command {
    // Global arguments only reach subcommands once the tree is built, so
    // render from a built copy and install into the original.
    let mut built = cmd.clone();
    built.build();
    install(cmd, &built, theme, None)
}

fn install(cmd: Command, built: &Command, theme: &Theme, parent: Option<&str>) -> Command {
    let path = match parent {
        Some(parent) => format!("{parent} {}", built.get_name()),
        None => built.get_name().to_string(),
    };
    let mut cmd = cmd
        .styles(theme.styles())
        .override_help(render(built, &path, theme));
    // Skips subcommands clap adds while building, such as `help`.
    for sub in built.get_subcommands() {
        if cmd.find_subcommand(sub.get_name()).is_none() {
            continue;
        }
        cmd = cmd.mut_subcommand(sub.get_name(), |orig| {
            install(orig, sub, theme, Some(&path))
        });
    }
    cmd
}

/// The help screen for one command. `path` is how it's invoked, e.g. `godo run`.
pub fn render(cmd: &Command, path: &str, theme: &Theme) -> StyledStr {
    let mut out = StyledStr::new();
    let t = theme;
    let options: Vec<&Arg> = cmd
        .get_arguments()
        .filter(|a| !a.is_positional() && !a.is_hide_set())
        .collect();
    let positionals: Vec<&Arg> = cmd
        .get_arguments()
        .filter(|a| a.is_positional() && !a.is_hide_set())
        .collect();
    let commands: Vec<&Command> = cmd.get_subcommands().filter(|c| !c.is_hide_set()).collect();

    if let Some(before) = cmd.get_before_long_help().or(cmd.get_before_help()) {
        let _ = writeln!(out, "{}\n", before.ansi());
    }
    let _ = writeln!(out, "{}{path}{:#}", t.title, t.title);
    if let Some(about) = cmd.get_long_about().or(cmd.get_about()) {
        for line in about.to_string().lines() {
            let _ = writeln!(out, "  {line}");
        }
    }

    let _ = writeln!(out, "\n{}Usage:{:#}", t.heading, t.heading);
    let _ = write!(out, "  $ {}{path}{:#}", t.title, t.title);
    if !options.is_empty() {
        let _ = write!(out, " {}[options]{:#}", t.placeholder, t.placeholder);
    }
    for arg in &positionals {
        let _ = write!(out, " {}", positional_usage(arg, t));
    }
    if !commands.is_empty() {
        let _ = write!(out, " {}<command>{:#}", t.placeholder, t.placeholder);
    }
    let _ = writeln!(out);
    if let (Some(flag), Some(sub)) = (options.first(), commands.first()) {
        let _ = writeln!(
            out,
            "  $ {}{path}{:#} {}{}{:#} {}{}{:#}",
            t.title,
            t.title,
            t.flag,
            short_or_long(flag),
            t.flag,
            t.command,
            sub.get_name(),
            t.command,
        );
    }

    if !positionals.is_empty() {
        let rows: Vec<Row> = positionals
            .iter()
            .map(|a| Row {
                name: (positional_name(a), t.placeholder),
                value: None,
                about: describe(a),
            })
            .collect();
        section(&mut out, "Arguments:", &rows, t);
    }
    if !options.is_empty() {
        let rows: Vec<Row> = options
            .iter()
            .map(|a| Row {
                name: (option_names(a), t.flag),
                value: value_placeholder(a),
                about: describe(a),
            })
            .collect();
        section(&mut out, "Options:", &rows, t);
    }
    if !commands.is_empty() {
        let rows: Vec<Row> = commands
            .iter()
            .map(|c| Row {
                name: (c.get_name_and_visible_aliases().join(", "), t.command),
                value: None,
                about: (
                    c.get_about().map(|a| a.to_string()).unwrap_or_default(),
                    None,
                ),
            })
            .collect();
        section(&mut out, "Commands:", &rows, t);
    }
    // `.ansi()` keeps the caller's colours; plain `{}` would drop them.
    if let Some(after) = cmd.get_after_long_help().or(cmd.get_after_help()) {
        let _ = writeln!(out, "\n{}", after.ansi());
    }
    out
}

struct Row {
    name: (String, Style),
    value: Option<String>,
    /// Description, and an optional note such as `[default: x]`.
    about: (String, Option<String>),
}

fn section(out: &mut StyledStr, heading: &str, rows: &[Row], t: &Theme) {
    let _ = writeln!(out, "\n{}{heading}{:#}", t.heading, t.heading);
    let width = rows.iter().map(row_width).max().unwrap_or(0);
    for row in rows {
        let (name, style) = &row.name;
        let _ = write!(out, "  {style}{name}{style:#}");
        if let Some(value) = &row.value {
            let _ = write!(out, " {}{value}{:#}", t.placeholder, t.placeholder);
        }
        let (about, note) = &row.about;
        let pad = width - row_width(row) + 4;
        let _ = write!(out, "{:pad$}{about}", "");
        if let Some(note) = note {
            let gap = if about.is_empty() { "" } else { " " };
            let _ = write!(out, "{gap}{}{note}{:#}", t.note, t.note);
        }
        let _ = writeln!(out);
    }
}

fn row_width(row: &Row) -> usize {
    row.name.0.chars().count() + row.value.as_ref().map_or(0, |v| v.chars().count() + 1)
}

fn option_names(arg: &Arg) -> String {
    let mut names: Vec<String> = Vec::new();
    for short in arg.get_short_and_visible_aliases().unwrap_or_default() {
        names.push(format!("-{short}"));
    }
    for long in arg.get_long_and_visible_aliases().unwrap_or_default() {
        names.push(format!("--{long}"));
    }
    names.join(", ")
}

fn short_or_long(arg: &Arg) -> String {
    match (arg.get_short(), arg.get_long()) {
        (Some(short), _) => format!("-{short}"),
        (None, Some(long)) => format!("--{long}"),
        (None, None) => arg.get_id().to_string(),
    }
}

fn value_placeholder(arg: &Arg) -> Option<String> {
    if !arg.get_action().takes_values() {
        return None;
    }
    let names = arg.get_value_names().map(|names| {
        names
            .iter()
            .map(|n| format!("<{n}>"))
            .collect::<Vec<_>>()
            .join(" ")
    });
    Some(names.unwrap_or_else(|| format!("<{}>", arg.get_id().as_str().to_uppercase())))
}

fn positional_name(arg: &Arg) -> String {
    let name = arg
        .get_value_names()
        .and_then(|n| n.first())
        .map(|n| n.to_string())
        .unwrap_or_else(|| arg.get_id().as_str().to_uppercase());
    let many = matches!(arg.get_action(), clap::ArgAction::Append);
    let dots = if many { "..." } else { "" };
    if arg.is_required_set() {
        format!("<{name}>{dots}")
    } else {
        format!("[{name}]{dots}")
    }
}

fn positional_usage(arg: &Arg, t: &Theme) -> String {
    format!(
        "{}{}{:#}",
        t.placeholder,
        positional_name(arg),
        t.placeholder
    )
}

fn describe(arg: &Arg) -> (String, Option<String>) {
    let about = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
    let mut notes = Vec::new();
    if !arg.is_positional() && arg.is_required_set() {
        notes.push("required".to_string());
    }
    let defaults: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|v| v.to_string_lossy().into_owned())
        .collect();
    if !defaults.is_empty() && arg.get_action().takes_values() {
        notes.push(format!("default: {}", defaults.join(", ")));
    }
    let note = (!notes.is_empty()).then(|| format!("[{}]", notes.join(", ")));
    (about, note)
}
