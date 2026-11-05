// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Minigames. Each one is a small state machine the app ticks, feeds keys
//! to, and draws onto a canvas until it's over.

mod echo;
mod highlow;
mod hop;
mod snack_rain;
mod which_way;

use ratatui::crossterm::event::KeyCode;
use ratatui::style::{Color, Style};

use crate::canvas::Canvas;

pub trait Game {
    fn tick(&mut self, dt: f32);
    fn key(&mut self, code: KeyCode);
    fn draw(&self, c: &mut Canvas);
    fn score(&self) -> u32;
    fn over(&self) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameKind {
    WhichWay,
    SnackRain,
    Hop,
    Echo,
    HighLow,
}

impl GameKind {
    pub const ALL: [GameKind; 5] = [
        GameKind::WhichWay,
        GameKind::SnackRain,
        GameKind::Hop,
        GameKind::Echo,
        GameKind::HighLow,
    ];

    /// Key for best scores in the save file.
    pub fn id(self) -> &'static str {
        match self {
            GameKind::WhichWay => "which-way",
            GameKind::SnackRain => "snack-rain",
            GameKind::Hop => "hop",
            GameKind::Echo => "echo",
            GameKind::HighLow => "high-low",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            GameKind::WhichWay => "Which Way?",
            GameKind::SnackRain => "Snack Rain",
            GameKind::Hop => "Hop!",
            GameKind::Echo => "Echo",
            GameKind::HighLow => "High or Low",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            GameKind::WhichWay => "guess where your pet will look",
            GameKind::SnackRain => "catch snacks, dodge rocks",
            GameKind::Hop => "jump the cacti, go far",
            GameKind::Echo => "repeat the growing pattern",
            GameKind::HighLow => "is the next card higher or lower?",
        }
    }

    pub fn controls(self) -> &'static str {
        match self {
            GameKind::WhichWay => "←/→ guess",
            GameKind::SnackRain => "←/→ move",
            GameKind::Hop => "space/↑ jump",
            GameKind::Echo => "arrows repeat the pattern",
            GameKind::HighLow => "↑ higher  ↓ lower",
        }
    }

    pub fn coins(self, score: u32) -> u32 {
        match self {
            GameKind::WhichWay => score * 2,
            GameKind::SnackRain => score,
            GameKind::Hop => score / 4,
            GameKind::Echo => score * 3,
            GameKind::HighLow => score * 2,
        }
    }

    pub fn start(self, face: String, seed: u64) -> Box<dyn Game> {
        match self {
            GameKind::WhichWay => Box::new(which_way::WhichWay::new(face, seed)),
            GameKind::SnackRain => Box::new(snack_rain::SnackRain::new(face, seed)),
            GameKind::Hop => Box::new(hop::Hop::new(face, seed)),
            GameKind::Echo => Box::new(echo::Echo::new(face, seed)),
            GameKind::HighLow => Box::new(highlow::HighLow::new(face, seed)),
        }
    }
}

pub(crate) fn style(c: Color) -> Style {
    Style::default().fg(c)
}

pub(crate) const PET: Color = Color::Rgb(255, 175, 205);
pub(crate) const GOOD: Color = Color::Rgb(140, 230, 160);
pub(crate) const BAD: Color = Color::Rgb(255, 110, 110);
pub(crate) const DIM: Color = Color::Rgb(110, 110, 130);
pub(crate) const GOLD: Color = Color::Rgb(255, 215, 110);

/// The top-left corner that centers a `w`×`h` field on the canvas.
pub(crate) fn origin(c: &Canvas, w: i32, h: i32) -> (i32, i32) {
    ((c.w - w) / 2, ((c.h - h) / 2).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every game ends if the player does nothing (or only mashes keys).
    #[test]
    fn every_game_ends() {
        for kind in GameKind::ALL {
            let mut g = kind.start("(^w^)".into(), 7);
            let keys = [KeyCode::Left, KeyCode::Up, KeyCode::Right, KeyCode::Down];
            for i in 0..20_000 {
                if g.over() {
                    break;
                }
                g.key(keys[i % 4]);
                g.tick(0.05);
            }
            assert!(g.over(), "{} never ended", kind.title());
            let mut c = Canvas::new(60, 20);
            g.draw(&mut c);
        }
    }
}
