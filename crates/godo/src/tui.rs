// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Pieces shared by `watch` and `history`: the palette of the C++ godo, the
//! terminal they draw on, and [`LogView`], which shows a job's output the
//! way a terminal would.

use std::collections::VecDeque;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};
use unicode_width::UnicodeWidthChar;

pub const BORDER: Color = Color::Indexed(240);
pub const SELECTED_BG: Color = Color::Indexed(24);
pub const HEADER: Color = Color::Rgb(0, 255, 128);
pub const GREEN: Color = Color::Rgb(0, 255, 0);
pub const RED: Color = Color::Rgb(255, 0, 0);
pub const YELLOW: Color = Color::Rgb(255, 255, 0);
pub const WHITE: Color = Color::Rgb(255, 255, 255);

/// A rounded grey pane with a grey title, as the C++ godo drew them.
pub fn pane(title: String) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(Span::styled(title, Style::new().fg(BORDER)))
}

/// `" GODO <NAME> "` in green, then the key hints.
pub fn header(name: &str, hints: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" GODO {name} "),
            Style::new().fg(HEADER).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" ({hints})")),
    ])
}

/// A dim line of key hints for the bottom row.
pub fn footer(text: &str) -> Line<'static> {
    Line::from(Span::styled(format!(" {text}"), Style::new().fg(BORDER)))
}

/// Logical lines kept by a [`LogView`]; older ones are dropped.
const MAX_LINES: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Cell {
    c: char,
    style: Style,
}

/// A job's output as a terminal would show it: colours, `\r` returning to
/// the start of the line, erase-line and cursor movement (so progress bars
/// redraw in place), wrapped to a width when drawn.
#[derive(Default)]
pub struct LogView {
    raw: String,
    /// Logical (unwrapped) lines; the cursor is on the last few.
    lines: VecDeque<Vec<Cell>>,
    /// Lines dropped from the front, so `cursor_line` stays absolute.
    dropped: usize,
    cursor_line: usize,
    cursor_col: usize,
    style: Style,
    escape: Option<String>,
    width: usize,
}

impl LogView {
    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn replace(&mut self, raw: &str) {
        let width = self.width;
        *self = Self {
            width,
            ..Self::default()
        };
        self.push(raw);
    }

    /// The text of every line kept, without styles or trailing spaces, and
    /// how many older lines were dropped before the first.
    pub fn plain(&self) -> (usize, Vec<String>) {
        let lines = self
            .lines
            .iter()
            .map(|line| {
                let text: String = line.iter().map(|cell| cell.c).collect();
                text.trim_end().to_string()
            })
            .collect();
        (self.dropped, lines)
    }

    pub fn set_width(&mut self, width: usize) {
        self.width = width;
    }

    pub fn push(&mut self, text: &str) {
        self.raw.push_str(text);
        for c in text.chars() {
            self.feed(c);
        }
        // Keep `raw` from outgrowing what's shown.
        if self.raw.len() > 64 << 20 {
            let keep = self.raw.len() - (32 << 20);
            let cut = (keep..self.raw.len())
                .find(|&i| self.raw.is_char_boundary(i))
                .unwrap_or(self.raw.len());
            self.raw.drain(..cut);
        }
    }

    /// Wrapped rows, counting the last line even when empty.
    pub fn total_rows(&self) -> usize {
        self.lines
            .iter()
            .map(|l| self.rows_of(l))
            .sum::<usize>()
            .max(1)
    }

    fn rows_of(&self, line: &[Cell]) -> usize {
        if self.width == 0 {
            return 1;
        }
        let width: usize = line.iter().map(|c| c.c.width().unwrap_or(0)).sum();
        width.div_ceil(self.width).max(1)
    }

    /// `height` rows ending `scroll` rows above the bottom.
    pub fn visible(&self, height: usize, scroll: usize) -> Vec<Line<'static>> {
        let want = height + scroll;
        let mut rows: Vec<Line<'static>> = Vec::new();
        for line in self.lines.iter().rev() {
            let mut wrapped = self.wrap(line);
            wrapped.reverse();
            rows.extend(wrapped);
            if rows.len() >= want {
                break;
            }
        }
        if rows.is_empty() {
            rows.push(Line::default());
        }
        rows.reverse();
        let end = rows.len().saturating_sub(scroll);
        let start = end.saturating_sub(height);
        rows.drain(start..end).collect()
    }

    fn wrap(&self, line: &[Cell]) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut text = String::new();
        let mut style = Style::default();
        let mut col = 0;
        for cell in line {
            let w = cell.c.width().unwrap_or(0);
            if self.width > 0 && col + w > self.width && col > 0 {
                if !text.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut text), style));
                }
                out.push(Line::from(std::mem::take(&mut spans)));
                col = 0;
            }
            if cell.style != style && !text.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut text), style));
            }
            style = cell.style;
            text.push(cell.c);
            col += w;
        }
        if !text.is_empty() {
            spans.push(Span::styled(text, style));
        }
        out.push(Line::from(spans));
        out
    }

    fn line_mut(&mut self) -> &mut Vec<Cell> {
        while self.dropped + self.lines.len() <= self.cursor_line {
            self.lines.push_back(Vec::new());
        }
        let i = self.cursor_line - self.dropped;
        &mut self.lines[i]
    }

    fn feed(&mut self, c: char) {
        if let Some(seq) = self.escape.as_mut() {
            seq.push(c);
            if escape_done(seq) {
                let seq = self.escape.take().expect("in an escape");
                self.apply_escape(&seq);
            }
            return;
        }
        match c {
            '\x1b' => self.escape = Some(String::new()),
            '\n' => self.newline(),
            '\r' => self.cursor_col = 0,
            '\x08' => self.cursor_col = self.cursor_col.saturating_sub(1),
            '\t' => {
                let next = (self.cursor_col / 8 + 1) * 8;
                while self.cursor_col < next {
                    self.put(' ');
                }
            }
            c if c.is_control() => {}
            c => self.put(c),
        }
    }

    fn newline(&mut self) {
        self.cursor_line += 1;
        self.cursor_col = 0;
        self.line_mut();
        while self.lines.len() > MAX_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    fn put(&mut self, c: char) {
        let cell = Cell {
            c,
            style: self.style,
        };
        let col = self.cursor_col;
        let line = self.line_mut();
        while line.len() < col {
            line.push(Cell {
                c: ' ',
                style: Style::default(),
            });
        }
        if col < line.len() {
            line[col] = cell;
        } else {
            line.push(cell);
        }
        self.cursor_col += 1;
    }

    fn apply_escape(&mut self, seq: &str) {
        let Some(body) = seq.strip_prefix('[') else {
            return;
        };
        let Some(command) = body.chars().last() else {
            return;
        };
        let params = &body[..body.len() - command.len_utf8()];
        let n = params.parse::<usize>().unwrap_or(1).max(1);
        match command {
            'm' => self.style = apply_sgr(self.style, params),
            // Erase in line: 0 to the end, 1 to the start, 2 all of it.
            'K' => {
                let col = self.cursor_col;
                let line = self.line_mut();
                match params {
                    "" | "0" => line.truncate(col),
                    "1" => {
                        for cell in line.iter_mut().take(col + 1) {
                            cell.c = ' ';
                        }
                    }
                    "2" => line.clear(),
                    _ => {}
                }
            }
            'A' => {
                let top = self.dropped;
                self.cursor_line = self.cursor_line.saturating_sub(n).max(top);
            }
            'B' => {
                self.cursor_line += n;
                self.line_mut();
            }
            'C' => self.cursor_col += n,
            'D' => self.cursor_col = self.cursor_col.saturating_sub(n),
            'G' => self.cursor_col = n - 1,
            // Cursor to the start of a line n up or down.
            'F' => {
                self.cursor_line = self.cursor_line.saturating_sub(n).max(self.dropped);
                self.cursor_col = 0;
            }
            'E' => {
                self.cursor_line += n;
                self.cursor_col = 0;
                self.line_mut();
            }
            // Everything else (clear screen, cursor shape, ...) is ignored.
            _ => {}
        }
    }
}

/// Whether an escape sequence (after the ESC) is complete: CSI ends at a
/// final byte, OSC at BEL or ST, anything else after one character.
fn escape_done(seq: &str) -> bool {
    match seq.chars().next() {
        None => false,
        Some('[') => seq.len() > 1 && seq.chars().last().is_some_and(|c| ('@'..='~').contains(&c)),
        Some(']') => seq.ends_with('\x07') || seq.ends_with("\x1b\\"),
        Some(_) => true,
    }
}

/// Applies an SGR sequence's parameters (`1;31`, `38;5;240`, ...).
pub fn apply_sgr(mut style: Style, params: &str) -> Style {
    if params.is_empty() {
        return Style::default();
    }
    let mut nums = params.split(';').map(|p| p.parse::<u16>().unwrap_or(0));
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

type Backend = ratatui::backend::CrosstermBackend<std::io::Stdout>;

/// The alternate screen in raw mode with mouse reporting, put back as it
/// was when dropped, even on panic.
pub struct Screen(pub ratatui::Terminal<Backend>);

impl Screen {
    pub fn enter() -> std::io::Result<Self> {
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

/// What a key or mouse event means in godo's full-screen views.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Quit,
    Back,
    Up,
    Down,
    PageUp,
    PageDown,
    HalfUp,
    HalfDown,
    Top,
    Bottom,
    WheelUp,
    WheelDown,
    Enter,
}

pub fn key(event: &ratatui::crossterm::event::Event) -> Option<Key> {
    use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
    match event {
        Event::Key(k) if k.kind != KeyEventKind::Release => {
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            Some(match k.code {
                KeyCode::Char('c') if ctrl => Key::Quit,
                KeyCode::Char('u') if ctrl => Key::HalfUp,
                KeyCode::Char('d') if ctrl => Key::HalfDown,
                KeyCode::Char('q') => Key::Quit,
                KeyCode::Esc => Key::Back,
                KeyCode::Up | KeyCode::Char('k') => Key::Up,
                KeyCode::Down | KeyCode::Char('j') => Key::Down,
                KeyCode::PageUp => Key::PageUp,
                KeyCode::PageDown => Key::PageDown,
                KeyCode::Home | KeyCode::Char('g') => Key::Top,
                KeyCode::End | KeyCode::Char('G') => Key::Bottom,
                KeyCode::Enter => Key::Enter,
                _ => return None,
            })
        }
        Event::Mouse(m) => match m.kind {
            MouseEventKind::ScrollUp => Some(Key::WheelUp),
            MouseEventKind::ScrollDown => Some(Key::WheelDown),
            _ => None,
        },
        _ => None,
    }
}

/// How a log pane scrolls: `scroll` rows up from the bottom, at most `max`.
pub fn scroll(scroll: usize, max: usize, page: usize, key: Key) -> usize {
    let half = (page / 2).max(1);
    let next = match key {
        Key::WheelUp => scroll + 3,
        Key::WheelDown => scroll.saturating_sub(3),
        Key::PageUp => scroll + page.max(1),
        Key::PageDown => scroll.saturating_sub(page.max(1)),
        Key::HalfUp => scroll + half,
        Key::HalfDown => scroll.saturating_sub(half),
        Key::Top => max,
        Key::Bottom => 0,
        _ => scroll,
    };
    next.min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(v: &LogView, height: usize) -> Vec<String> {
        v.visible(height, 0)
            .iter()
            .map(|l| l.to_string().trim_end().to_string())
            .collect()
    }

    #[test]
    fn wraps_and_keeps_colours_across_lines() {
        let mut v = LogView::default();
        v.set_width(5);
        v.push("\x1b[31mabcdefg\nhi\x1b[0m!");
        let lines = v.visible(10, 0);
        let rows: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        assert_eq!(rows, ["abcde", "fg", "hi!"]);
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::Indexed(1)));
        assert_eq!(
            lines[2].spans[0].style.fg,
            Some(Color::Indexed(1)),
            "red carries on"
        );
        assert_eq!(lines[2].spans[1].style.fg, None, "reset");
    }

    #[test]
    fn carriage_return_redraws_a_progress_bar_in_place() {
        let mut v = LogView::default();
        v.set_width(40);
        v.push("Compiling\n[=   ] 25%\r[==  ] 50%\r[====] 100%\ndone\n");
        assert_eq!(text(&v, 10), ["Compiling", "[====] 100%", "done", ""]);
    }

    #[test]
    fn erase_line_and_cursor_up_redraw_several_lines() {
        // As bazel and cargo do: go up, clear, write the status again.
        let mut v = LogView::default();
        v.set_width(40);
        v.push("a: 1/3\nb: 1/3\n");
        v.push("\x1b[2A\x1b[2Ka: 3/3\n\x1b[2Kb: 2/3\n");
        assert_eq!(text(&v, 10), ["a: 3/3", "b: 2/3", ""]);
        v.push("x\x1b[1G\x1b[Kyz\x08\x08Y");
        assert_eq!(text(&v, 10).last().unwrap(), "Yz");
    }

    #[test]
    fn a_shorter_overwrite_leaves_the_rest_like_a_terminal() {
        let mut v = LogView::default();
        v.set_width(40);
        v.push("downloading 100%\rdone");
        assert_eq!(text(&v, 10), ["doneloading 100%"]);
    }

    #[test]
    fn unknown_escapes_are_dropped() {
        let mut v = LogView::default();
        v.set_width(3);
        v.push("ab\x1b[?25lcdef\x1b]0;title\x07\n");
        v.set_width(10);
        assert_eq!(text(&v, 10), ["abcdef", ""]);
    }

    #[test]
    fn scrolling_counts_wrapped_rows() {
        let mut v = LogView::default();
        v.set_width(4);
        v.push("aaaaaaaa\nbb\n");
        assert_eq!(v.total_rows(), 4);
        let rows: Vec<String> = v.visible(2, 2).iter().map(|l| l.to_string()).collect();
        assert_eq!(rows, ["aaaa", "aaaa"]);
    }

    #[test]
    fn old_lines_are_dropped_past_the_limit() {
        let mut v = LogView::default();
        v.set_width(80);
        for i in 0..MAX_LINES + 10 {
            v.push(&format!("{i}\n"));
        }
        assert!(v.lines.len() <= MAX_LINES);
        assert_eq!(text(&v, 2), [format!("{}", MAX_LINES + 9), String::new()]);
        // Cursor movement can't reach dropped lines.
        v.push("\x1b[99999Az");
    }

    #[test]
    fn sgr_parses_256_and_truecolour() {
        let s = apply_sgr(Style::default(), "1;38;5;240;48;2;1;2;3");
        assert_eq!(s.fg, Some(Color::Indexed(240)));
        assert_eq!(s.bg, Some(Color::Rgb(1, 2, 3)));
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert_eq!(apply_sgr(s, ""), Style::default());
    }

    #[test]
    fn scroll_keys_stay_in_range() {
        assert_eq!(scroll(0, 100, 20, Key::PageUp), 20);
        assert_eq!(scroll(15, 100, 20, Key::HalfDown), 5);
        assert_eq!(scroll(5, 100, 20, Key::PageDown), 0);
        assert_eq!(scroll(0, 100, 20, Key::Top), 100);
        assert_eq!(scroll(90, 100, 20, Key::PageUp), 100);
        assert_eq!(scroll(50, 100, 20, Key::Bottom), 0);
        assert_eq!(scroll(0, 100, 20, Key::WheelUp), 3);
    }
}
