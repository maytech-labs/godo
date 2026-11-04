// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! `godo watch`: a live dashboard of jobs, drawn like the C++ godo's.
//!
//! The header is `GODO WATCH` in green. Below it, a 30-column TARGETS pane
//! lists every `directive target` started since watch opened (plus the
//! daemon itself), with a status dot: green running, red failed, yellow
//! succeeded, yellow ○ nothing yet. Beside it, the LOGS pane shows the
//! selected target's output, wrapped and following the end.
//!
//! Keys, as in the C++ godo: q or Esc quits. ↑/k and ↓/j move between
//! targets, or scroll once scrolled; k scrolls whenever there are logs.
//! PgUp and PgDn scroll 20 lines, and so does the mouse wheel, one line at
//! a time. It redraws 30 times a second and asks the daemon for jobs every
//! tenth frame.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use unicode_width::UnicodeWidthChar;

use crate::logs::Tailer;
use crate::proto::ProcessInfo;

const SIDEBAR_WIDTH: u16 = 30;
const FPS: u64 = 30;
/// Ask the daemon for jobs every this many frames.
const POLL_EVERY: u64 = 10;
const PAGE: usize = 20;

const BORDER: Color = Color::Indexed(240);
const SELECTED_BG: Color = Color::Indexed(24);
const HEADER: Color = Color::Rgb(0, 255, 128);
const GREEN: Color = Color::Rgb(0, 255, 0);
const RED: Color = Color::Rgb(255, 0, 0);
const YELLOW: Color = Color::Rgb(255, 255, 0);
const WHITE: Color = Color::Rgb(255, 255, 255);

/// Where watch gets its jobs and their logs.
pub trait Source {
    /// Every job the daemon knows, or `None` if it can't be reached.
    fn processes(&mut self) -> Option<Vec<ProcessInfo>>;
    fn log_path(&self, id: u64) -> PathBuf;
}

pub enum Input {
    Quit,
    Up,
    /// `k`: scrolls whenever there are logs, else moves up.
    K,
    Down,
    PageUp,
    PageDown,
    WheelUp,
    WheelDown,
}

/// What a selected target's logs were, kept while another is selected.
struct Saved {
    raw: String,
    tailer: Tailer,
}

pub struct Watcher<S: Source> {
    source: S,
    /// Jobs that started before this are left out, as in the C++ godo.
    start_secs: u64,
    tick: u64,
    processes: Vec<ProcessInfo>,
    /// Every key seen, in first-seen order, so finished targets stay.
    seen: Vec<String>,
    selected: String,
    scroll: usize,
    pub quit: bool,
    logs: LogView,
    tailer: Tailer,
    /// Which jobs were running at the last poll, to announce their exits.
    running: HashMap<u64, bool>,
    saved: HashMap<String, Saved>,
    /// The log pane's inner height at the last draw.
    log_height: usize,
}

impl<S: Source> Watcher<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            start_secs: crate::proto::now_ms() / 1000,
            tick: 0,
            processes: Vec::new(),
            seen: Vec::new(),
            selected: String::new(),
            scroll: 0,
            quit: false,
            logs: LogView::default(),
            tailer: Tailer::default(),
            running: HashMap::new(),
            saved: HashMap::new(),
            log_height: 0,
        }
    }

    /// Target keys: current ones in daemon order, then any seen earlier.
    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for p in &self.processes {
            let key = p.key();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        for key in &self.seen {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
        keys
    }

    fn group(&self, key: &str) -> Vec<&ProcessInfo> {
        self.processes.iter().filter(|p| p.key() == key).collect()
    }

    fn log_paths(&self, key: &str) -> Vec<PathBuf> {
        self.group(key)
            .iter()
            .map(|p| self.source.log_path(p.id))
            .collect()
    }

    fn max_scroll(&self) -> usize {
        self.logs.total_lines().saturating_sub(self.log_height)
    }

    pub fn input(&mut self, input: Input) {
        let max = self.max_scroll();
        match input {
            Input::Quit => self.quit = true,
            Input::Up if self.scroll > 0 => self.scroll = (self.scroll + 1).min(max),
            Input::K if self.scroll > 0 || !self.logs.is_empty() => {
                self.scroll = (self.scroll + 1).min(max)
            }
            Input::Up | Input::K => self.select_offset(-1),
            Input::Down | Input::WheelDown if self.scroll > 0 => self.scroll -= 1,
            Input::Down => self.select_offset(1),
            Input::WheelDown => {}
            Input::WheelUp => self.scroll = (self.scroll + 1).min(max),
            Input::PageUp => self.scroll = (self.scroll + PAGE).min(max),
            Input::PageDown => self.scroll = self.scroll.saturating_sub(PAGE),
        }
    }

    /// Moves the selection, wrapping around at either end.
    fn select_offset(&mut self, by: isize) {
        let keys = self.keys();
        if keys.is_empty() {
            return;
        }
        let at = keys.iter().position(|k| *k == self.selected);
        let next = match (at, by < 0) {
            (Some(0), true) | (None, true) => keys.len() - 1,
            (Some(i), true) => i - 1,
            (Some(i), false) if i + 1 < keys.len() => i + 1,
            _ => 0,
        };
        self.select(keys[next].clone());
    }

    fn select(&mut self, key: String) {
        if key == self.selected {
            return;
        }
        if !self.selected.is_empty() {
            let saved = Saved {
                raw: self.logs.raw().to_string(),
                tailer: self.tailer.clone(),
            };
            self.saved.insert(std::mem::take(&mut self.selected), saved);
        }
        self.scroll = 0;
        match self.saved.remove(&key) {
            Some(saved) => {
                self.tailer = saved.tailer;
                self.logs.replace(&saved.raw);
            }
            None => {
                self.tailer = Tailer::default();
                let paths = self.log_paths(&key);
                let text = self.tailer.read_all(&paths);
                self.logs.replace(&text);
            }
        }
        self.selected = key;
    }

    /// One frame's work: poll the daemon now and then, and follow the
    /// selected target's logs.
    pub fn tick(&mut self) {
        if self.tick.is_multiple_of(POLL_EVERY) {
            self.poll();
        }
        self.tick += 1;
        if self.selected.is_empty() {
            return;
        }
        let paths = self.log_paths(&self.selected);
        let new = self.tailer.read_new(&paths);
        if !new.is_empty() {
            self.append(&new);
        }
    }

    fn poll(&mut self) {
        let Some(mut processes) = self.source.processes() else {
            return;
        };
        let start = self.start_secs;
        processes.retain(|p| p.id == 0 || p.start_ms / 1000 >= start);
        // Read output that arrived before the job ended, then announce it.
        let selected_paths: Vec<PathBuf> = processes
            .iter()
            .filter(|p| p.key() == self.selected)
            .map(|p| self.source.log_path(p.id))
            .collect();
        let mut exits = Vec::new();
        for p in &processes {
            let was_running = self.running.insert(p.id, p.running);
            if was_running == Some(true) && !p.running && p.key() == self.selected {
                exits.push(p.exit_code.unwrap_or(-1));
            }
        }
        self.processes = processes;
        for key in self.keys() {
            if !self.seen.contains(&key) {
                self.seen.push(key);
            }
        }
        if !exits.is_empty() {
            let new = self.tailer.read_new(&selected_paths);
            self.append(&new);
            for code in exits {
                self.append(&format!(
                    "\n\x1b[1;33mProcess exited with code: {code}\x1b[0m\n"
                ));
            }
        }
        if self.selected.is_empty()
            && let Some(first) = self.keys().into_iter().next()
        {
            self.select(first);
        }
    }

    /// Adds output, keeping a scrolled view on the same lines.
    fn append(&mut self, text: &str) {
        let before = self.logs.total_lines();
        self.logs.push(text);
        if self.scroll > 0 {
            self.scroll += self.logs.total_lines() - before;
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let header = Line::from(vec![
            Span::styled(
                " GODO WATCH ",
                Style::new().fg(HEADER).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" (q to quit, arrows to navigate)"),
        ]);
        frame.render_widget(header, Rect { height: 1, ..area });

        let body_height = area.height.saturating_sub(2);
        let side_width = SIDEBAR_WIDTH.min(area.width);
        let side = Rect::new(area.x, area.y + 1, side_width, body_height);
        let main = Rect::new(
            area.x + side_width,
            area.y + 1,
            area.width - side_width,
            body_height,
        );
        let pane = |title: String| {
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(BORDER))
                .title(Span::styled(title, Style::new().fg(BORDER)))
        };

        let inner_side = side_width.saturating_sub(2) as usize;
        let targets: Vec<Line> = self
            .keys()
            .into_iter()
            .map(|key| self.target_line(&key, inner_side))
            .collect();
        frame.render_widget(
            Paragraph::new(targets).block(pane(" TARGETS ".into())),
            side,
        );

        // Like the C++ godo, logs wrap two columns short of the pane.
        self.logs.set_width(main.width.saturating_sub(4) as usize);
        self.log_height = body_height.saturating_sub(2) as usize;
        self.scroll = self.scroll.min(self.max_scroll());
        let title = if self.selected.is_empty() {
            " LOGS ".to_string()
        } else {
            format!(" LOGS: {} ", self.selected)
        };
        let content: Vec<Line> = if !self.logs.is_empty() {
            self.logs.visible(self.log_height, self.scroll)
        } else if self.selected.is_empty() {
            vec![Line::raw("Select a target to view logs")]
        } else {
            vec![Line::raw(format!(
                "No logs available for {}",
                self.selected
            ))]
        };
        frame.render_widget(Paragraph::new(content).block(pane(title)), main);
    }

    fn target_line(&self, key: &str, width: usize) -> Line<'static> {
        let newest = self.group(key).into_iter().max_by_key(|p| p.id);
        let (dot, colour) = match newest {
            Some(p) if p.running => (" ● ", GREEN),
            Some(p) if p.exit_code == Some(0) => (" ● ", YELLOW),
            Some(_) => (" ● ", RED),
            None => (" ○ ", YELLOW),
        };
        let dot = Span::styled(dot, Style::new().fg(colour));
        if key == self.selected {
            // Columns, not characters: wide characters take two.
            let used = 2 + 3 + unicode_width::UnicodeWidthStr::width(key);
            let pad = " ".repeat(width.saturating_sub(used));
            Line::from(vec![
                Span::raw("> "),
                dot,
                Span::styled(key.to_string(), Style::new().fg(WHITE)),
                Span::raw(pad),
            ])
            .style(Style::new().bg(SELECTED_BG))
        } else {
            Line::from(vec![Span::raw("  "), dot, Span::raw(key.to_string())])
        }
    }
}

/// Text with ANSI colours, turned into styled lines wrapped to a width as
/// it arrives. Colours carry across lines, as in a terminal.
#[derive(Default)]
pub struct LogView {
    raw: String,
    width: usize,
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    text: String,
    text_style: Style,
    column: usize,
    style: Style,
    escape: Option<String>,
}

impl LogView {
    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Wrapped lines, counting the line being written.
    pub fn total_lines(&self) -> usize {
        self.lines.len() + 1
    }

    pub fn replace(&mut self, raw: &str) {
        let width = self.width;
        *self = Self {
            width,
            ..Self::default()
        };
        self.push(raw);
    }

    pub fn set_width(&mut self, width: usize) {
        if width != self.width {
            self.width = width;
            let raw = std::mem::take(&mut self.raw);
            self.replace(&raw);
        }
    }

    pub fn push(&mut self, text: &str) {
        self.raw.push_str(text);
        for c in text.chars() {
            self.feed(c);
        }
    }

    /// `height` lines ending `scroll` lines above the bottom.
    pub fn visible(&self, height: usize, scroll: usize) -> Vec<Line<'static>> {
        let mut all: Vec<Line<'static>> = self.lines.clone();
        let mut current = self.spans.clone();
        if !self.text.is_empty() {
            current.push(Span::styled(self.text.clone(), self.text_style));
        }
        all.push(Line::from(current));
        let end = all.len().saturating_sub(scroll);
        let start = end.saturating_sub(height);
        all.drain(start..end).collect()
    }

    fn feed(&mut self, c: char) {
        if let Some(seq) = self.escape.as_mut() {
            seq.push(c);
            if escape_done(seq) {
                let seq = self.escape.take().expect("in an escape");
                if let Some(params) = seq.strip_prefix('[').and_then(|s| s.strip_suffix('m')) {
                    self.style = apply_sgr(self.style, params);
                }
            }
            return;
        }
        match c {
            '\x1b' => self.escape = Some(String::new()),
            '\n' => self.break_line(),
            '\t' => {
                let spaces = 8 - self.column % 8;
                for _ in 0..spaces {
                    self.put(' ', 1);
                }
            }
            c if c.is_control() => {}
            c => {
                let w = c.width().unwrap_or(0);
                self.put(c, w);
            }
        }
    }

    fn put(&mut self, c: char, w: usize) {
        if self.width > 0 && self.column + w > self.width && self.column > 0 {
            self.break_line();
        }
        if self.style != self.text_style && !self.text.is_empty() {
            let text = std::mem::take(&mut self.text);
            self.spans.push(Span::styled(text, self.text_style));
        }
        self.text_style = self.style;
        self.text.push(c);
        self.column += w;
    }

    fn break_line(&mut self) {
        let mut spans = std::mem::take(&mut self.spans);
        if !self.text.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut self.text),
                self.text_style,
            ));
        }
        self.lines.push(Line::from(spans));
        self.column = 0;
    }
}

/// Whether an escape sequence (after the ESC) is complete: CSI ends at a
/// final byte, OSC at BEL or ST, anything else after one character.
fn escape_done(seq: &str) -> bool {
    let mut chars = seq.chars();
    match chars.next() {
        None => false,
        Some('[') => seq.len() > 1 && seq.chars().last().is_some_and(|c| ('@'..='~').contains(&c)),
        Some(']') => seq.ends_with('\x07') || seq.ends_with("\x1b\\"),
        Some(_) => true,
    }
}

/// Applies an SGR sequence's parameters (`1;31`, `38;5;240`, ...).
fn apply_sgr(mut style: Style, params: &str) -> Style {
    let mut nums = params
        .split(';')
        .map(|p| p.parse::<u16>().unwrap_or(0))
        .peekable();
    if params.is_empty() {
        return Style::default();
    }
    while let Some(n) = nums.next() {
        style = match n {
            0 => Style::default(),
            1 => style.add_modifier(Modifier::BOLD),
            2 => style.add_modifier(Modifier::DIM),
            3 => style.add_modifier(Modifier::ITALIC),
            4 => style.add_modifier(Modifier::UNDERLINED),
            5 => style.add_modifier(Modifier::SLOW_BLINK),
            7 => style.add_modifier(Modifier::REVERSED),
            9 => style.add_modifier(Modifier::CROSSED_OUT),
            21 | 22 => style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => style.remove_modifier(Modifier::ITALIC),
            24 => style.remove_modifier(Modifier::UNDERLINED),
            25 => style.remove_modifier(Modifier::SLOW_BLINK),
            27 => style.remove_modifier(Modifier::REVERSED),
            29 => style.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 => style.fg(Color::Indexed((n - 30) as u8)),
            39 => style.fg(Color::Reset),
            40..=47 => style.bg(Color::Indexed((n - 40) as u8)),
            49 => style.bg(Color::Reset),
            90..=97 => style.fg(Color::Indexed((n - 90 + 8) as u8)),
            100..=107 => style.bg(Color::Indexed((n - 100 + 8) as u8)),
            38 | 48 => {
                let colour = match nums.next() {
                    Some(5) => nums.next().map(|i| Color::Indexed(i as u8)),
                    Some(2) => {
                        let (r, g, b) = (nums.next(), nums.next(), nums.next());
                        Some(Color::Rgb(
                            r.unwrap_or(0) as u8,
                            g.unwrap_or(0) as u8,
                            b.unwrap_or(0) as u8,
                        ))
                    }
                    _ => None,
                };
                match (n, colour) {
                    (38, Some(c)) => style.fg(c),
                    (48, Some(c)) => style.bg(c),
                    _ => style,
                }
            }
            _ => style,
        };
    }
    style
}

/// The daemon, through its socket and log directory.
pub struct DaemonSource {
    home: crate::proto::Home,
}

impl Source for DaemonSource {
    fn processes(&mut self) -> Option<Vec<ProcessInfo>> {
        crate::client::connect(&self.home)?;
        crate::client::processes(&self.home).ok()
    }

    fn log_path(&self, id: u64) -> PathBuf {
        self.home.log(id)
    }
}

/// Runs the dashboard until q, starting the daemon if it isn't running.
pub fn run(home: crate::proto::Home) -> clapsmith::Result {
    use ratatui::crossterm::event::{
        self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
    };

    crate::client::connect_or_start(&home)?;
    let mut watcher = Watcher::new(DaemonSource { home });
    let mut terminal = Screen::enter()?;
    let frame_time = Duration::from_millis(1000 / FPS);
    let mut next_tick = Instant::now();
    while !watcher.quit {
        if Instant::now() >= next_tick {
            watcher.tick();
            next_tick += frame_time;
            if next_tick < Instant::now() {
                next_tick = Instant::now() + frame_time;
            }
        }
        terminal.0.draw(|f| watcher.draw(f))?;
        let wait = next_tick.saturating_duration_since(Instant::now());
        if !event::poll(wait)? {
            continue;
        }
        let input = match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => Some(Input::Quit),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(Input::Quit)
                }
                KeyCode::Up => Some(Input::Up),
                KeyCode::Char('k') => Some(Input::K),
                KeyCode::Down | KeyCode::Char('j') => Some(Input::Down),
                KeyCode::PageUp => Some(Input::PageUp),
                KeyCode::PageDown => Some(Input::PageDown),
                _ => None,
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => Some(Input::WheelUp),
                MouseEventKind::ScrollDown => Some(Input::WheelDown),
                _ => None,
            },
            _ => None,
        };
        if let Some(input) = input {
            watcher.input(input);
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

type Backend = ratatui::backend::CrosstermBackend<std::io::Stdout>;

/// The alternate screen in raw mode with mouse reporting, put back as it
/// was when dropped, even on panic.
struct Screen(ratatui::Terminal<Backend>);

impl Screen {
    fn enter() -> std::io::Result<Self> {
        use ratatui::crossterm::{event::EnableMouseCapture, execute, terminal};
        terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, terminal::EnterAlternateScreen, EnableMouseCapture)?;
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            Self::restore();
            previous(info);
        }));
        let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
        term.hide_cursor()?;
        Ok(Self(term))
    }

    fn restore() {
        use ratatui::crossterm::{event::DisableMouseCapture, execute, terminal};
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            DisableMouseCapture,
            terminal::LeaveAlternateScreen,
            ratatui::crossterm::cursor::Show
        );
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        Self::restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    struct Fake {
        processes: Vec<ProcessInfo>,
        dir: crate::testdir::TestDir,
    }

    impl Source for Fake {
        fn processes(&mut self) -> Option<Vec<ProcessInfo>> {
            Some(self.processes.clone())
        }
        fn log_path(&self, id: u64) -> PathBuf {
            self.dir.path().join(format!("{id}.log"))
        }
    }

    fn job(
        id: u64,
        directive: &str,
        target: &str,
        running: bool,
        code: Option<i32>,
    ) -> ProcessInfo {
        ProcessInfo {
            id,
            pid: 100 + id as u32,
            command: String::new(),
            target: target.into(),
            directive: directive.into(),
            running,
            exit_code: code,
            start_ms: crate::proto::now_ms() + 5_000,
            end_ms: None,
        }
    }

    fn fake(test: &str, processes: Vec<ProcessInfo>) -> Watcher<Fake> {
        let dir = crate::testdir::TestDir::new(&format!("watch-{test}"));
        Watcher::new(Fake { processes, dir })
    }

    fn log(w: &Watcher<Fake>, id: u64, text: &str) {
        let mut writer = crate::logs::LogWriter::open(&w.source.log_path(id)).unwrap();
        writer.write("out", text).unwrap();
    }

    fn screen(w: &mut Watcher<Fake>, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut t = Terminal::new(TestBackend::new(width, height)).unwrap();
        t.draw(|f| w.draw(f)).unwrap();
        t.backend().buffer().clone()
    }

    fn row(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn layout_matches_the_cpp_dashboard() {
        let mut w = fake("layout", vec![job(1, "build", "web", true, None)]);
        log(&w, 1, "compiling\n");
        w.tick();
        let buf = screen(&mut w, 80, 12);
        assert!(row(&buf, 0).starts_with(" GODO WATCH  (q to quit, arrows to navigate)"));
        assert_eq!(buf[(1, 0)].fg, HEADER);
        assert!(buf[(1, 0)].modifier.contains(Modifier::BOLD));
        assert!(row(&buf, 1).starts_with("╭ TARGETS ─"), "{}", row(&buf, 1));
        assert!(
            row(&buf, 1)[..].contains("╭ LOGS: build web ─"),
            "{}",
            row(&buf, 1)
        );
        assert_eq!(buf[(0, 1)].fg, BORDER);
        // The selected target: "> ● build web" on blue, padded across the pane.
        assert!(
            row(&buf, 2).starts_with("│>  ● build web"),
            "{}",
            row(&buf, 2)
        );
        assert_eq!(buf[(4, 2)].fg, GREEN);
        assert_eq!(buf[(6, 2)].fg, WHITE);
        assert_eq!(buf[(28, 2)].bg, SELECTED_BG);
        // The sidebar is 30 columns; the last row is left blank.
        assert_eq!(buf[(29, 2)].symbol(), "│");
        assert_eq!(buf[(30, 1)].symbol(), "╭");
        assert!(row(&buf, 11).trim().is_empty());
        assert!(row(&buf, 10).starts_with("╰"));
        // Short logs start at the top, as in the C++ godo; long ones follow
        // the end (see scrolling_moves_through_logs_and_back).
        assert!(
            row(&buf, 2).contains("│compiling"),
            "{}",
            row(&buf, 2).trim()
        );
    }

    #[test]
    fn wide_target_names_keep_the_selection_aligned() {
        let mut w = fake("wide", vec![job(1, "构建", "网站", true, None)]);
        w.tick();
        let buf = screen(&mut w, 60, 6);
        // The highlight still ends at the pane's edge and the border stays put.
        assert_eq!(buf[(28, 2)].bg, SELECTED_BG, "{}", row(&buf, 2));
        assert_eq!(buf[(29, 2)].symbol(), "│", "{}", row(&buf, 2));
        // ratatui clips at the border anyway, so check the line itself.
        assert_eq!(w.target_line("构建 网站", 28).width(), 28);
    }

    #[test]
    fn status_dots_follow_the_newest_job() {
        let mut w = fake(
            "dots",
            vec![
                job(1, "build", "ok", false, Some(0)),
                job(2, "build", "bad", false, Some(2)),
                job(3, "test", "web", false, Some(1)),
                job(4, "test", "web", true, None),
            ],
        );
        w.tick();
        let buf = screen(&mut w, 60, 10);
        let dot = |y: u16| buf[(4, y)].fg;
        assert!(row(&buf, 2).contains("build ok"));
        assert_eq!(dot(2), YELLOW);
        assert_eq!(dot(3), RED);
        assert_eq!(dot(4), GREEN, "test web's newest job is running");
    }

    #[test]
    fn finished_targets_stay_listed_with_a_hollow_dot() {
        let mut w = fake("seen", vec![job(1, "dev", "api", true, None)]);
        w.tick();
        w.source.processes.clear();
        for _ in 0..POLL_EVERY {
            w.tick();
        }
        let buf = screen(&mut w, 60, 8);
        assert!(row(&buf, 2).contains("○ dev api"), "{}", row(&buf, 2));
    }

    #[test]
    fn jobs_from_before_watch_opened_are_left_out() {
        let mut old = job(1, "build", "old", false, Some(0));
        old.start_ms = 1_000;
        let mut daemon = job(0, "system", "daemon", true, None);
        daemon.start_ms = 1_000;
        let mut w = fake("filter", vec![daemon, old]);
        w.tick();
        assert_eq!(w.keys(), ["system daemon"]);
    }

    #[test]
    fn arrows_move_between_targets_and_wrap() {
        let mut w = fake(
            "nav",
            vec![job(1, "a", "x", true, None), job(2, "b", "y", true, None)],
        );
        w.tick();
        assert_eq!(w.selected, "a x");
        w.input(Input::Down);
        assert_eq!(w.selected, "b y");
        w.input(Input::Down);
        assert_eq!(w.selected, "a x", "wraps to the top");
        w.input(Input::Up);
        assert_eq!(w.selected, "b y", "wraps to the bottom");
    }

    #[test]
    fn scrolling_moves_through_logs_and_back() {
        let mut w = fake("scroll", vec![job(1, "a", "x", true, None)]);
        log(
            &w,
            1,
            &(0..50).map(|i| format!("line {i}\n")).collect::<String>(),
        );
        w.tick();
        let _ = screen(&mut w, 60, 12);
        w.input(Input::K);
        assert_eq!(w.scroll, 1, "k scrolls when there are logs");
        w.input(Input::Up);
        assert_eq!(w.scroll, 2, "up scrolls once scrolled");
        w.input(Input::PageUp);
        assert_eq!(w.scroll, 22);
        let buf = screen(&mut w, 60, 12);
        assert!(row(&buf, 2).contains("line 21"), "{}", row(&buf, 2));
        w.input(Input::PageDown);
        w.input(Input::Down);
        w.input(Input::Down);
        assert_eq!(w.scroll, 0);
        assert_eq!(w.selected, "a x", "down only scrolls until the bottom");
        w.input(Input::PageUp);
        w.input(Input::PageUp);
        w.input(Input::PageUp);
        assert!(w.scroll <= w.max_scroll(), "can't scroll past the top");
    }

    #[test]
    fn new_output_is_followed_and_exits_announced() {
        let mut w = fake("follow", vec![job(1, "a", "x", true, None)]);
        w.tick();
        log(&w, 1, "first\n");
        w.tick();
        assert!(w.logs.raw().contains("first"));
        log(&w, 1, "last\n");
        w.source.processes = vec![job(1, "a", "x", false, Some(3))];
        for _ in 0..POLL_EVERY {
            w.tick();
        }
        assert!(
            w.logs
                .raw()
                .ends_with("last\n\n\x1b[1;33mProcess exited with code: 3\x1b[0m\n"),
            "{:?}",
            w.logs.raw()
        );
    }

    #[test]
    fn switching_back_keeps_a_targets_logs() {
        let mut w = fake(
            "switch",
            vec![job(1, "a", "x", true, None), job(2, "b", "y", true, None)],
        );
        log(&w, 1, "from x\n");
        w.tick();
        w.input(Input::Down);
        log(&w, 1, "more x\n");
        w.input(Input::Down);
        w.tick();
        assert_eq!(w.logs.raw(), "from x\nmore x\n");
    }

    #[test]
    fn empty_states_say_what_to_do() {
        let mut w = fake("empty", vec![]);
        let buf = screen(&mut w, 70, 6);
        assert!(row(&buf, 2).contains("Select a target to view logs"));
        let mut w = fake("nologs", vec![job(1, "a", "x", true, None)]);
        w.tick();
        let buf = screen(&mut w, 70, 6);
        assert!(
            row(&buf, 2).contains("No logs available for a x"),
            "{}",
            row(&buf, 2)
        );
    }

    #[test]
    fn log_view_wraps_and_keeps_colours_across_lines() {
        let mut v = LogView::default();
        v.set_width(5);
        v.push("\x1b[31mabcdefg\nhi\x1b[0m!");
        let lines = v.visible(10, 0);
        let text: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        assert_eq!(text, ["abcde", "fg", "hi!"]);
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::Indexed(1)));
        assert_eq!(
            lines[2].spans[0].style.fg,
            Some(Color::Indexed(1)),
            "red carries on"
        );
        assert_eq!(lines[2].spans[1].style.fg, None, "reset");
    }

    #[test]
    fn log_view_rewraps_on_resize_and_drops_other_escapes() {
        let mut v = LogView::default();
        v.set_width(3);
        v.push("ab\x1b[2Kcdef\x1b]0;title\x07\n");
        assert_eq!(v.total_lines(), 3);
        v.set_width(10);
        let text: Vec<String> = v.visible(10, 0).iter().map(|l| l.to_string()).collect();
        assert_eq!(text, ["abcdef", ""]);
    }

    #[test]
    fn sgr_parses_256_and_truecolour() {
        let s = apply_sgr(Style::default(), "1;38;5;240;48;2;1;2;3");
        assert_eq!(s.fg, Some(Color::Indexed(240)));
        assert_eq!(s.bg, Some(Color::Rgb(1, 2, 3)));
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert_eq!(apply_sgr(s, ""), Style::default());
    }
}
