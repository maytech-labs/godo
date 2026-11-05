// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! `godo watch`: a live dashboard of jobs, drawn like the C++ godo's.
//!
//! The header is `GODO WATCH` in green. Below it, a 30-column TARGETS pane
//! lists each `directive target` whose newest job is running or started in
//! the last day (`--since`, `--all`), plus the daemon itself, with a status
//! dot: green running, red failed, yellow succeeded, yellow ○ no jobs left.
//! Beside it, the LOGS pane shows the selected target's output as a
//! terminal would (progress bars redraw in place), following the end.
//!
//! Keys: ↑/↓ or k/j choose a target. PgUp/PgDn, Ctrl+U/Ctrl+D, and the
//! mouse wheel scroll the logs; g/G or Home/End jump to the top or bottom.
//! q, Esc, or Ctrl+C quits. It redraws 30 times a second and asks the
//! daemon for jobs every tenth frame.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::logs::Tailer;
use crate::proto::ProcessInfo;
use crate::tui::{self, GREEN, Key, LogView, RED, SELECTED_BG, WHITE, YELLOW};

const SIDEBAR_WIDTH: u16 = 30;
const FPS: u64 = 30;
/// Ask the daemon for jobs every this many frames.
const POLL_EVERY: u64 = 10;
/// The default window: targets with a job in the last day.
pub const DEFAULT_SINCE: Duration = Duration::from_secs(24 * 60 * 60);

/// Where watch gets its jobs and their logs.
pub trait Source {
    /// Every job the daemon knows, or `None` if it can't be reached.
    fn processes(&mut self) -> Option<Vec<ProcessInfo>>;
    fn log_path(&self, id: u64) -> PathBuf;
}

/// What a selected target's logs were, kept while another is selected.
struct Saved {
    raw: String,
    tailer: Tailer,
}

pub struct Watcher<S: Source> {
    source: S,
    /// Targets whose newest job started before this (ms) are left out,
    /// unless it's still running. `None` shows everything.
    cutoff: Option<u64>,
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
    /// `since`: how far back to show targets; `None` for all of history.
    pub fn new(source: S, since: Option<Duration>) -> Self {
        let now = crate::proto::now_ms();
        Self {
            source,
            cutoff: since.map(|d| now.saturating_sub(d.as_millis() as u64)),
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
        self.logs.total_rows().saturating_sub(self.log_height)
    }

    pub fn input(&mut self, key: Key) {
        match key {
            Key::Quit | Key::Back => self.quit = true,
            Key::Up => self.select_offset(-1),
            Key::Down => self.select_offset(1),
            Key::Enter => {}
            scrolling => {
                self.scroll =
                    tui::scroll(self.scroll, self.max_scroll(), self.log_height, scrolling)
            }
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

    /// Jobs in scope: the daemon, and every job of a target whose newest
    /// job is running or started after the cutoff.
    fn in_scope(&self, processes: Vec<ProcessInfo>) -> Vec<ProcessInfo> {
        let Some(cutoff) = self.cutoff else {
            return processes;
        };
        let mut newest: HashMap<String, &ProcessInfo> = HashMap::new();
        for p in &processes {
            let entry = newest.entry(p.key()).or_insert(p);
            if p.id > entry.id {
                *entry = p;
            }
        }
        let keep: Vec<String> = newest
            .into_iter()
            .filter(|(_, p)| p.id == 0 || p.running || p.start_ms >= cutoff)
            .map(|(k, _)| k)
            .collect();
        processes
            .into_iter()
            .filter(|p| keep.contains(&p.key()))
            .collect()
    }

    fn poll(&mut self) {
        let Some(processes) = self.source.processes() else {
            return;
        };
        let processes = self.in_scope(processes);
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
        let before = self.logs.total_rows();
        self.logs.push(text);
        if self.scroll > 0 {
            self.scroll += self.logs.total_rows().saturating_sub(before);
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(
            tui::header("WATCH", "q to quit, arrows to navigate"),
            Rect { height: 1, ..area },
        );
        let body_height = area.height.saturating_sub(2);
        let side_width = SIDEBAR_WIDTH.min(area.width);
        let side = Rect::new(area.x, area.y + 1, side_width, body_height);
        let main = Rect::new(
            area.x + side_width,
            area.y + 1,
            area.width - side_width,
            body_height,
        );

        // The TARGETS list scrolls to keep the selection in view.
        let keys = self.keys();
        let rows = body_height.saturating_sub(2) as usize;
        let at = keys.iter().position(|k| *k == self.selected).unwrap_or(0);
        let first = if keys.len() <= rows {
            0
        } else {
            at.saturating_sub(rows.saturating_sub(1) / 2)
                .min(keys.len() - rows)
        };
        let inner_side = side_width.saturating_sub(2) as usize;
        let targets: Vec<Line> = keys
            .iter()
            .skip(first)
            .take(rows)
            .map(|key| self.target_line(key, inner_side))
            .collect();
        let mut block = tui::pane(" TARGETS ".into());
        let more = Style::new().fg(tui::BORDER);
        if first > 0 {
            block = block.title(Line::styled(format!(" ▲ {first} more "), more).right_aligned());
        }
        let below = keys.len().saturating_sub(first + rows);
        if below > 0 {
            block =
                block.title_bottom(Line::styled(format!(" ▼ {below} more "), more).right_aligned());
        }
        frame.render_widget(Paragraph::new(targets).block(block), side);

        // Like the C++ godo, logs wrap two columns short of the pane.
        self.logs.set_width(main.width.saturating_sub(4) as usize);
        self.log_height = body_height.saturating_sub(2) as usize;
        self.scroll = self.scroll.min(self.max_scroll());
        let mut title = if self.selected.is_empty() {
            " LOGS ".to_string()
        } else {
            format!(" LOGS: {} ", self.selected)
        };
        if self.scroll > 0 {
            title.push_str(&format!("↑{} ", self.scroll));
        }
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
        frame.render_widget(Paragraph::new(content).block(tui::pane(title)), main);

        if area.height > 2 {
            let footer = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
            frame.render_widget(
                tui::footer(
                    "↑↓/jk target · PgUp/PgDn ^U/^D wheel scroll · g/G top/bottom · q quit",
                ),
                footer,
            );
        }
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
        // Jobs `godo mcp` started for an agent are marked.
        let by_agent = newest.is_some_and(|p| p.origin == crate::proto::Origin::Agent);
        let mark = Span::styled(
            if by_agent { " agent" } else { "" },
            Style::new().fg(tui::BORDER),
        );
        if key == self.selected {
            // Columns, not characters: wide characters take two.
            let used = 2 + 3 + unicode_width::UnicodeWidthStr::width(key) + mark.content.len();
            let pad = " ".repeat(width.saturating_sub(used));
            Line::from(vec![
                Span::raw("> "),
                dot,
                Span::styled(key.to_string(), Style::new().fg(WHITE)),
                mark,
                Span::raw(pad),
            ])
            .style(Style::new().bg(SELECTED_BG))
        } else {
            Line::from(vec![Span::raw("  "), dot, Span::raw(key.to_string()), mark])
        }
    }
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
pub fn run(home: crate::proto::Home, since: Option<Duration>) -> clapsmith::Result {
    use ratatui::crossterm::event;

    crate::client::connect_or_start(&home)?;
    let mut watcher = Watcher::new(DaemonSource { home }, since);
    let mut screen = tui::Screen::enter()?;
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
        screen.0.draw(|f| watcher.draw(f))?;
        let wait = next_tick.saturating_duration_since(Instant::now());
        if !event::poll(wait)? {
            continue;
        }
        if let Some(key) = tui::key(&event::read()?) {
            watcher.input(key);
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

/// Parses `90s`, `30m`, `2h`, `7d`, or a bare number of seconds.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let (number, unit) = text
        .find(|c: char| !c.is_ascii_digit())
        .map_or((text, "s"), |i| text.split_at(i));
    let bad = || format!("`{text}` isn't a duration; try 30m, 2h, or 7d");
    let n: u64 = number.parse().map_err(|_| bad())?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => return Err(bad()),
    };
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::{BORDER, HEADER};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

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
            start_ms: crate::proto::now_ms(),
            end_ms: None,
            origin: Default::default(),
            client: None,
            project: None,
            pty: true,
        }
    }

    fn fake(test: &str, processes: Vec<ProcessInfo>) -> Watcher<Fake> {
        let dir = crate::testdir::TestDir::new(&format!("watch-{test}"));
        Watcher::new(Fake { processes, dir }, Some(DEFAULT_SINCE))
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
            row(&buf, 1).contains("╭ LOGS: build web ─"),
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
        assert_eq!(buf[(29, 2)].symbol(), "│");
        assert_eq!(buf[(30, 1)].symbol(), "╭");
        assert!(row(&buf, 10).starts_with("╰"));
        // Short logs start at the top, as in the C++ godo.
        assert!(
            row(&buf, 2).contains("│compiling"),
            "{}",
            row(&buf, 2).trim()
        );
        // The bottom row lists the keys.
        assert!(row(&buf, 11).contains("q quit"), "{}", row(&buf, 11));
    }

    #[test]
    fn wide_target_names_keep_the_selection_aligned() {
        let mut w = fake("wide", vec![job(1, "构建", "网站", true, None)]);
        w.tick();
        let buf = screen(&mut w, 60, 6);
        assert_eq!(buf[(28, 2)].bg, SELECTED_BG, "{}", row(&buf, 2));
        assert_eq!(buf[(29, 2)].symbol(), "│", "{}", row(&buf, 2));
        // ratatui clips at the border anyway, so check the line itself.
        assert_eq!(w.target_line("构建 网站", 28).width(), 28);
    }

    #[test]
    fn targets_an_agent_ran_last_are_marked() {
        let mut agent = job(2, "build", "web", true, None);
        agent.origin = crate::proto::Origin::Agent;
        let mut w = fake(
            "agent",
            vec![
                job(1, "build", "web", false, Some(0)),
                agent,
                job(3, "test", "web", true, None),
            ],
        );
        w.tick();
        let buf = screen(&mut w, 80, 8);
        let rows: Vec<String> = (2..4).map(|y| row(&buf, y)).collect();
        assert!(
            rows.iter().any(|r| r.contains("build web agent")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|r| r.contains("test web") && !r.contains("agent")),
            "{rows:?}"
        );
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
    fn scope_is_recent_or_running_targets() {
        let day = 24 * 3600 * 1000;
        let mut old = job(1, "build", "old", false, Some(0));
        old.start_ms -= 2 * day;
        let mut old_running = job(2, "dev", "server", true, None);
        old_running.start_ms -= 2 * day;
        // A target counts if its newest job is recent, even with old ones.
        let mut web_old = job(3, "test", "web", false, Some(0));
        web_old.start_ms -= 2 * day;
        let web_new = job(4, "test", "web", false, Some(0));
        let mut daemon = job(0, "system", "daemon", true, None);
        daemon.start_ms -= 2 * day;
        let all = vec![daemon, old, old_running, web_old, web_new];
        let mut w = fake("scope", all.clone());
        w.tick();
        assert_eq!(w.keys(), ["system daemon", "dev server", "test web"]);
        assert_eq!(w.group("test web").len(), 2, "with all of its jobs");
        let mut everything = Watcher::new(
            Fake {
                processes: all,
                dir: crate::testdir::TestDir::new("watch-scope-all"),
            },
            None,
        );
        everything.tick();
        assert_eq!(everything.keys().len(), 4);
    }

    #[test]
    fn arrows_and_jk_choose_targets_and_wrap() {
        let mut w = fake(
            "nav",
            vec![job(1, "a", "x", true, None), job(2, "b", "y", true, None)],
        );
        log(&w, 1, "some logs\n");
        w.tick();
        assert_eq!(w.selected, "a x");
        w.input(Key::Down);
        assert_eq!(w.selected, "b y");
        w.input(Key::Down);
        assert_eq!(w.selected, "a x", "wraps to the top");
        w.input(Key::Up);
        assert_eq!(w.selected, "b y", "wraps to the bottom; k/↑ never scroll");
        assert_eq!(w.scroll, 0);
    }

    #[test]
    fn scroll_keys_move_through_logs() {
        let mut w = fake("scroll", vec![job(1, "a", "x", true, None)]);
        log(
            &w,
            1,
            &(0..50).map(|i| format!("line {i}\n")).collect::<String>(),
        );
        w.tick();
        let _ = screen(&mut w, 60, 12);
        w.input(Key::PageUp);
        assert_eq!(w.scroll, 8, "a page is the pane's height");
        w.input(Key::HalfDown);
        assert_eq!(w.scroll, 4);
        w.input(Key::Top);
        let buf = screen(&mut w, 60, 12);
        assert!(row(&buf, 2).contains("line 0"), "{}", row(&buf, 2));
        assert!(
            row(&buf, 1).contains("↑"),
            "the title shows how far up: {}",
            row(&buf, 1)
        );
        w.input(Key::Bottom);
        assert_eq!(w.scroll, 0);
        w.input(Key::WheelUp);
        assert_eq!(w.scroll, 3);
        assert_eq!(w.selected, "a x");
    }

    #[test]
    fn a_long_target_list_scrolls_with_the_selection() {
        let jobs: Vec<ProcessInfo> = (1..=20)
            .map(|i| job(i, "build", &format!("t{i:02}"), true, None))
            .collect();
        let mut w = fake("longlist", jobs);
        w.tick();
        let buf = screen(&mut w, 60, 10);
        // 6 rows fit; the rest are counted below.
        assert!(row(&buf, 8).contains("▼ 14 more"), "{}", row(&buf, 8));
        for _ in 0..12 {
            w.input(Key::Down);
        }
        let buf = screen(&mut w, 60, 10);
        let shown: String = (2..8).map(|y| row(&buf, y)).collect();
        assert!(
            shown.contains("> ") && shown.contains("build t13"),
            "{shown}"
        );
        assert!(row(&buf, 1).contains("▲"), "{}", row(&buf, 1));
        assert!(row(&buf, 8).contains("▼"), "{}", row(&buf, 8));
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
        w.input(Key::Down);
        log(&w, 1, "more x\n");
        w.input(Key::Down);
        w.tick();
        assert_eq!(w.logs.raw(), "from x\nmore x\n");
    }

    #[test]
    fn progress_bars_redraw_in_place() {
        let mut w = fake("progress", vec![job(1, "a", "x", true, None)]);
        log(&w, 1, "Building\n[=  ] 33%\r[== ] 66%\r[===] 100%\n");
        w.tick();
        let buf = screen(&mut w, 60, 8);
        assert!(row(&buf, 3).contains("[===] 100%"), "{}", row(&buf, 3));
        assert!(!(2..6).any(|y| row(&buf, y).contains("33%")));
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
    fn durations_parse() {
        assert_eq!(parse_duration("90").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("7d").unwrap(), Duration::from_secs(604_800));
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("5y").is_err());
    }
}
