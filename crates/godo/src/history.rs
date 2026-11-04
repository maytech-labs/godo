// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! `godo history` in a terminal: every job, newest first, drawn like the
//! C++ godo's. Enter opens a job's logs (followed live if it's running);
//! q or Esc goes back.
//!
//! Keys in the list: ↑/↓ or k/j move, PgUp/PgDn a page, g/G or Home/End
//! the first and last job, Enter opens. In the logs: ↑/↓ or k/j scroll a
//! line, PgUp/PgDn, Ctrl+U/Ctrl+D, and the wheel scroll more, g/G jump.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::logs::Tailer;
use crate::proto::ProcessInfo;
use crate::tui::{self, BORDER, Key, LogView, WHITE};

/// Column widths, as the C++ godo had them.
const WIDTHS: [usize; 5] = [6, 20, 15, 10, 20];
const HEADERS: [&str; 5] = ["ID", "TARGET", "DIRECTIVE", "STATUS", "START TIME"];
const SELECTED_BG: Color = Color::Indexed(12);
const HEADER_FG: Color = Color::Indexed(6);
/// How often the list is refreshed.
const REFRESH: Duration = Duration::from_secs(1);

pub trait Source {
    /// Every job, oldest first.
    fn jobs(&mut self) -> Vec<ProcessInfo>;
    fn log_path(&self, id: u64) -> PathBuf;
}

struct Open {
    id: u64,
    logs: LogView,
    tailer: Tailer,
    scroll: usize,
}

pub struct History<S: Source> {
    source: S,
    /// Newest first.
    jobs: Vec<ProcessInfo>,
    selected: usize,
    open: Option<Open>,
    pub quit: bool,
    /// Rows of the table and of the log pane at the last draw.
    table_rows: usize,
    log_height: usize,
}

impl<S: Source> History<S> {
    pub fn new(source: S) -> Self {
        let mut h = Self {
            source,
            jobs: Vec::new(),
            selected: 0,
            open: None,
            quit: false,
            table_rows: 0,
            log_height: 0,
        };
        h.refresh();
        h
    }

    /// Reloads the list, keeping the same job selected.
    pub fn refresh(&mut self) {
        let keep = self.jobs.get(self.selected).map(|p| p.id);
        let mut jobs = self.source.jobs();
        jobs.reverse();
        self.jobs = jobs;
        self.selected = keep
            .and_then(|id| self.jobs.iter().position(|p| p.id == id))
            .unwrap_or(0)
            .min(self.jobs.len().saturating_sub(1));
    }

    /// Follows the open job's log, if it's still being written.
    pub fn tick(&mut self) {
        if let Some(open) = self.open.as_mut() {
            let new = open.tailer.read_new(&[self.source.log_path(open.id)]);
            if !new.is_empty() {
                let before = open.logs.total_rows();
                open.logs.push(&new);
                if open.scroll > 0 {
                    open.scroll += open.logs.total_rows().saturating_sub(before);
                }
            }
        }
    }

    pub fn input(&mut self, key: Key) {
        if let Some(open) = self.open.as_mut() {
            let max = open.logs.total_rows().saturating_sub(self.log_height);
            match key {
                Key::Quit | Key::Back => self.open = None,
                Key::Up => open.scroll = (open.scroll + 1).min(max),
                Key::Down => open.scroll = open.scroll.saturating_sub(1),
                Key::Enter => {}
                other => open.scroll = tui::scroll(open.scroll, max, self.log_height, other),
            }
            return;
        }
        let last = self.jobs.len().saturating_sub(1);
        let page = self.table_rows.max(1);
        match key {
            Key::Quit | Key::Back => self.quit = true,
            Key::Up | Key::WheelUp => self.selected = self.selected.saturating_sub(1),
            Key::Down | Key::WheelDown => self.selected = (self.selected + 1).min(last),
            Key::PageUp | Key::HalfUp => self.selected = self.selected.saturating_sub(page),
            Key::PageDown | Key::HalfDown => self.selected = (self.selected + page).min(last),
            Key::Top => self.selected = 0,
            Key::Bottom => self.selected = last,
            Key::Enter => self.open_selected(),
        }
    }

    fn open_selected(&mut self) {
        let Some(job) = self.jobs.get(self.selected) else {
            return;
        };
        let mut tailer = Tailer::default();
        let text = tailer.read_all(&[self.source.log_path(job.id)]);
        let mut logs = LogView::default();
        logs.push(&text);
        self.open = Some(Open {
            id: job.id,
            logs,
            tailer,
            scroll: 0,
        });
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let body = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(2),
        );
        let footer = Rect::new(
            area.x,
            area.y + area.height.saturating_sub(1),
            area.width,
            1,
        );
        if let Some(open) = self.open.as_mut() {
            frame.render_widget(tui::header("HISTORY", "q to back"), area);
            open.logs.set_width(body.width.saturating_sub(2) as usize);
            self.log_height = body.height.saturating_sub(2) as usize;
            let max = open.logs.total_rows().saturating_sub(self.log_height);
            open.scroll = open.scroll.min(max);
            let content = if open.logs.is_empty() {
                vec![Line::raw(format!("No logs found for process {}", open.id))]
            } else {
                open.logs.visible(self.log_height, open.scroll)
            };
            let mut title = format!(" LOGS: {} ", open.id);
            if open.scroll > 0 {
                title.push_str(&format!("↑{} ", open.scroll));
            }
            frame.render_widget(Paragraph::new(content).block(tui::pane(title)), body);
            if area.height > 2 {
                frame.render_widget(
                    tui::footer("↑↓/jk PgUp/PgDn ^U/^D wheel scroll · g/G top/bottom · q back"),
                    footer,
                );
            }
            return;
        }

        frame.render_widget(
            tui::header(
                "HISTORY",
                "q to quit, arrows to navigate, enter to view logs",
            ),
            area,
        );
        // Top border, header, separator, and bottom border take four rows.
        self.table_rows = (body.height.saturating_sub(2) as usize).saturating_sub(4);
        let rows = self.table_rows;
        let offset = if self.jobs.len() <= rows {
            0
        } else {
            self.selected
                .saturating_sub(rows / 2)
                .min(self.jobs.len() - rows)
        };
        let border = Style::new().fg(BORDER);
        let mut lines = vec![rule('┌', '┬', '┐', border)];
        let header: Vec<String> = HEADERS.iter().map(|h| h.to_string()).collect();
        lines.push(cells(
            &header,
            border,
            Style::new().fg(HEADER_FG).add_modifier(Modifier::BOLD),
        ));
        lines.push(rule('├', '┼', '┤', border));
        for (i, job) in self.jobs.iter().enumerate().skip(offset).take(rows) {
            let selected = i == self.selected;
            let prefix = if selected { "> " } else { "  " };
            let row = [
                format!("{prefix}{}", job.id),
                job.target.clone(),
                job.directive.clone(),
                crate::client::status(job).to_string(),
                crate::proto::local_time(job.start_ms),
            ];
            let style = if selected {
                Style::new()
                    .bg(SELECTED_BG)
                    .fg(WHITE)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(cells(&row, border, style));
        }
        lines.push(rule('└', '┴', '┘', border));
        if self.jobs.is_empty() {
            lines.push(Line::raw("No jobs yet."));
        }
        frame.render_widget(
            Paragraph::new(lines).block(tui::pane(" COMMAND HISTORY ".into())),
            body,
        );
        if area.height > 2 {
            frame.render_widget(
                tui::footer("↑↓/jk move · PgUp/PgDn page · g/G first/last · enter logs · q quit"),
                footer,
            );
        }
    }
}

fn rule(left: char, middle: char, right: char, style: Style) -> Line<'static> {
    let parts: Vec<String> = WIDTHS.iter().map(|w| "─".repeat(w + 2)).collect();
    Line::styled(
        format!("{left}{}{right}", parts.join(&middle.to_string())),
        style,
    )
}

/// One table row: each cell padded (or cut) to its column, one space on
/// either side.
fn cells(values: &[String], border: Style, style: Style) -> Line<'static> {
    let mut spans = vec![Span::styled("│", border)];
    for (value, width) in values.iter().zip(WIDTHS) {
        let mut text = String::new();
        for c in value.chars() {
            if text.width() + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) > width {
                break;
            }
            text.push(c);
        }
        let pad = width - text.width();
        spans.push(Span::styled(format!(" {text}{} ", " ".repeat(pad)), style));
        spans.push(Span::styled("│", border));
    }
    Line::from(spans)
}

/// The daemon's jobs, or its saved history when it isn't running.
pub struct DaemonSource {
    home: crate::proto::Home,
}

impl Source for DaemonSource {
    fn jobs(&mut self) -> Vec<ProcessInfo> {
        crate::client::jobs(&self.home).unwrap_or_default()
    }

    fn log_path(&self, id: u64) -> PathBuf {
        self.home.log(id)
    }
}

pub fn run(home: crate::proto::Home) -> clapsmith::Result {
    use ratatui::crossterm::event;

    let mut history = History::new(DaemonSource { home });
    let mut screen = tui::Screen::enter()?;
    let mut refreshed = Instant::now();
    while !history.quit {
        if refreshed.elapsed() >= REFRESH {
            history.refresh();
            refreshed = Instant::now();
        }
        history.tick();
        screen.0.draw(|f| history.draw(f))?;
        if event::poll(Duration::from_millis(50))?
            && let Some(key) = tui::key(&event::read()?)
        {
            history.input(key);
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    struct Fake {
        jobs: Vec<ProcessInfo>,
        dir: crate::testdir::TestDir,
    }

    impl Source for Fake {
        fn jobs(&mut self) -> Vec<ProcessInfo> {
            self.jobs.clone()
        }
        fn log_path(&self, id: u64) -> PathBuf {
            self.dir.path().join(format!("{id}.log"))
        }
    }

    fn job(id: u64, target: &str, running: bool, code: Option<i32>) -> ProcessInfo {
        ProcessInfo {
            id,
            pid: 1,
            command: String::new(),
            target: target.into(),
            directive: "build".into(),
            running,
            exit_code: code,
            start_ms: 0,
            end_ms: None,
        }
    }

    fn history(test: &str, jobs: Vec<ProcessInfo>) -> History<Fake> {
        let dir = crate::testdir::TestDir::new(&format!("history-{test}"));
        History::new(Fake { jobs, dir })
    }

    fn screen(h: &mut History<Fake>, w: u16, rows: u16) -> ratatui::buffer::Buffer {
        let mut t = Terminal::new(TestBackend::new(w, rows)).unwrap();
        t.draw(|f| h.draw(f)).unwrap();
        t.backend().buffer().clone()
    }

    fn row(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn the_list_looks_like_the_cpp_one() {
        let mut h = history(
            "list",
            vec![
                job(1, "web", false, Some(0)),
                job(2, "api", false, Some(2)),
                job(3, "db", true, None),
                job(4, "old", false, None),
            ],
        );
        let buf = screen(&mut h, 100, 14);
        assert!(
            row(&buf, 0)
                .starts_with(" GODO HISTORY  (q to quit, arrows to navigate, enter to view logs)")
        );
        assert!(row(&buf, 1).starts_with("╭ COMMAND HISTORY ─"));
        assert!(row(&buf, 2).starts_with("│┌────────┬"), "{}", row(&buf, 2));
        assert!(
            row(&buf, 3).contains("│ ID     │ TARGET "),
            "{}",
            row(&buf, 3)
        );
        assert_eq!(buf[(3, 3)].fg, HEADER_FG);
        // Newest first, the first selected.
        assert!(row(&buf, 5).contains("│ > 4    │ old"), "{}", row(&buf, 5));
        assert!(row(&buf, 5).contains("Stopped"), "{}", row(&buf, 5));
        assert_eq!(buf[(4, 5)].bg, SELECTED_BG);
        assert!(row(&buf, 6).contains("Running") && row(&buf, 7).contains("Failed"));
        assert!(row(&buf, 8).contains("Success"));
        assert!(row(&buf, 9).contains("└────────┴"), "{}", row(&buf, 9));
        assert!(row(&buf, 13).contains("enter logs"));
    }

    #[test]
    fn moving_is_clamped_and_long_lists_scroll() {
        let jobs: Vec<ProcessInfo> = (1..=30)
            .map(|i| job(i, &format!("t{i}"), false, Some(0)))
            .collect();
        let mut h = history("scroll", jobs);
        h.input(Key::Up);
        assert_eq!(h.selected, 0, "no wrapping at the top");
        let _ = screen(&mut h, 100, 14);
        h.input(Key::Bottom);
        assert_eq!(h.selected, 29);
        let buf = screen(&mut h, 100, 14);
        let shown: String = (5..13).map(|y| row(&buf, y)).collect();
        assert!(shown.contains("> 1 "), "the oldest job is in view: {shown}");
        h.input(Key::PageUp);
        assert_eq!(h.selected, 29 - h.table_rows);
    }

    #[test]
    fn enter_opens_logs_and_q_goes_back() {
        let mut h = history("logs", vec![job(7, "web", true, None)]);
        let mut w = crate::logs::LogWriter::open(&h.source.log_path(7)).unwrap();
        w.write("out", "hello\nworking 10%\rworking 90%\n").unwrap();
        h.input(Key::Enter);
        let buf = screen(&mut h, 60, 10);
        assert!(row(&buf, 0).starts_with(" GODO HISTORY  (q to back)"));
        assert!(row(&buf, 1).starts_with("╭ LOGS: 7 "));
        assert!(row(&buf, 2).contains("hello") && row(&buf, 3).contains("working 90%"));
        // A running job's log is followed.
        w.write("out", "more\n").unwrap();
        h.tick();
        let buf = screen(&mut h, 60, 10);
        assert!(row(&buf, 4).contains("more"), "{}", row(&buf, 4));
        h.input(Key::Quit);
        assert!(h.open.is_none() && !h.quit, "q goes back first");
        h.input(Key::Quit);
        assert!(h.quit);
    }

    #[test]
    fn a_job_without_logs_says_so() {
        let mut h = history("nologs", vec![job(3, "web", false, Some(0))]);
        h.input(Key::Enter);
        let buf = screen(&mut h, 60, 8);
        assert!(row(&buf, 2).contains("No logs found for process 3"));
    }

    #[test]
    fn refreshing_keeps_the_selected_job() {
        let mut h = history(
            "refresh",
            vec![job(1, "a", false, Some(0)), job(2, "b", false, Some(0))],
        );
        h.input(Key::Down);
        assert_eq!(h.jobs[h.selected].id, 1);
        h.source.jobs.push(job(3, "c", true, None));
        h.refresh();
        assert_eq!(
            h.jobs[h.selected].id, 1,
            "still job 1, though it moved down"
        );
    }

    #[test]
    fn long_cells_are_cut_to_their_column() {
        let line = cells(
            &[
                "1".into(),
                "a-very-long-target-name-indeed".into(),
                "d".into(),
                "s".into(),
                "t".into(),
            ],
            Style::default(),
            Style::default(),
        );
        let text = line.to_string();
        assert!(text.contains("│ a-very-long-target-n │"), "{text}");
    }
}
