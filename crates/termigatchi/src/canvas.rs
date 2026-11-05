// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! A tiny character grid to draw sprites on, turned into ratatui lines.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    cells: Vec<(char, Style)>,
}

impl Canvas {
    pub fn new(w: u16, h: u16) -> Canvas {
        Canvas {
            w: w as i32,
            h: h as i32,
            cells: vec![(' ', Style::default()); w as usize * h as usize],
        }
    }

    pub fn put(&mut self, x: i32, y: i32, ch: char, style: Style) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.cells[(y * self.w + x) as usize] = (ch, style);
        }
    }

    pub fn text(&mut self, x: i32, y: i32, s: &str, style: Style) {
        for (i, ch) in s.chars().enumerate() {
            self.put(x + i as i32, y, ch, style);
        }
    }

    pub fn text_centered(&mut self, y: i32, s: &str, style: Style) {
        let x = (self.w - s.chars().count() as i32) / 2;
        self.text(x, y, s, style);
    }

    /// Draws `lines` with their top-left at (x, y). Spaces are transparent.
    pub fn sprite<S: AsRef<str>>(&mut self, x: i32, y: i32, lines: &[S], style: Style) {
        for (dy, line) in lines.iter().enumerate() {
            for (dx, ch) in line.as_ref().chars().enumerate() {
                if ch != ' ' {
                    self.put(x + dx as i32, y + dy as i32, ch, style);
                }
            }
        }
    }

    /// Applies `style` on top of every cell.
    pub fn tint(&mut self, style: Style) {
        for cell in &mut self.cells {
            cell.1 = cell.1.patch(style);
        }
    }

    pub fn lines(&self) -> Vec<Line<'static>> {
        (0..self.h)
            .map(|y| {
                let row = &self.cells[(y * self.w) as usize..((y + 1) * self.w) as usize];
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut text = String::new();
                let mut style = row.first().map(|c| c.1).unwrap_or_default();
                for &(ch, s) in row {
                    if s != style {
                        spans.push(Span::styled(std::mem::take(&mut text), style));
                        style = s;
                    }
                    text.push(ch);
                }
                spans.push(Span::styled(text, style));
                Line::from(spans)
            })
            .collect()
    }
}
