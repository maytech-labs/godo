// SPDX-License-Identifier: LicenseRef-REAL-1.0

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use ratatui::crossterm::event::KeyCode;
use ratatui::style::Color;

use super::{BAD, DIM, GOLD, GOOD, Game, PET, origin, style};
use crate::canvas::Canvas;

const MAX_STREAK: u32 = 12;
const SUITS: [char; 4] = ['♠', '♥', '♦', '♣'];

#[derive(Clone, Copy)]
struct Card {
    rank: u8,
    suit: usize,
}

impl Card {
    fn label(self) -> &'static str {
        [
            "A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K",
        ][self.rank as usize - 1]
    }
}

/// Guess whether the next card is higher or lower. Ties are free.
pub struct HighLow {
    rng: StdRng,
    face: String,
    card: Card,
    streak: u32,
    /// The next card, whether the guess was right, and time left to show it.
    reveal: Option<(Card, bool, f32)>,
    lost: bool,
}

impl HighLow {
    pub fn new(face: String, seed: u64) -> HighLow {
        let mut rng = StdRng::seed_from_u64(seed);
        let card = draw_card(&mut rng);
        HighLow {
            rng,
            face,
            card,
            streak: 0,
            reveal: None,
            lost: false,
        }
    }
}

fn draw_card(rng: &mut StdRng) -> Card {
    Card {
        rank: rng.random_range(1..=13),
        suit: rng.random_range(0..4),
    }
}

fn card_lines(card: Option<Card>) -> [String; 5] {
    match card {
        Some(c) => {
            let l = c.label();
            let s = SUITS[c.suit];
            [
                ".-----.".into(),
                format!("|{l:<5}|"),
                format!("|  {s}  |"),
                format!("|{l:>5}|"),
                "'-----'".into(),
            ]
        }
        None => [
            ".-----.".into(),
            "|░░░░░|".into(),
            "|░ ? ░|".into(),
            "|░░░░░|".into(),
            "'-----'".into(),
        ],
    }
}

fn card_color(card: Option<Card>) -> Color {
    match card {
        Some(c) if c.suit == 1 || c.suit == 2 => BAD,
        Some(_) => Color::White,
        None => DIM,
    }
}

impl Game for HighLow {
    fn tick(&mut self, dt: f32) {
        if let Some((next, right, t)) = &mut self.reveal {
            *t -= dt;
            if *t <= 0.0 {
                if *right {
                    self.card = *next;
                    self.reveal = None;
                } else {
                    self.lost = true;
                }
            }
        }
    }

    fn key(&mut self, code: KeyCode) {
        if self.reveal.is_some() || self.over() {
            return;
        }
        let higher = match code {
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('k') => true,
            KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('j') => false,
            _ => return,
        };
        let next = draw_card(&mut self.rng);
        let right = next.rank == self.card.rank || (next.rank > self.card.rank) == higher;
        if right {
            self.streak += 1;
        }
        self.reveal = Some((next, right, 1.0));
    }

    fn draw(&self, c: &mut Canvas) {
        let (ox, oy) = origin(c, 24, 10);
        c.text_centered(
            oy,
            &format!("Streak {} / {MAX_STREAK}", self.streak),
            style(GOLD),
        );
        let next = self.reveal.map(|r| r.0);
        c.sprite(
            ox + 1,
            oy + 2,
            &card_lines(Some(self.card)),
            style(card_color(Some(self.card))),
        );
        c.sprite(ox + 16, oy + 2, &card_lines(next), style(card_color(next)));
        c.text(ox + 10, oy + 4, "→", style(DIM));
        let (msg, col) = match self.reveal {
            None if self.over() => ("A perfect run!".to_string(), GOLD),
            None => (format!("{} Higher (↑) or lower (↓)?", self.face), PET),
            Some((n, true, _)) if n.rank == self.card.rank => ("Same! Free pass.".into(), GOOD),
            Some((_, true, _)) => ("Correct!".into(), GOOD),
            Some((_, false, _)) => ("Aww, no!".into(), BAD),
        };
        c.text_centered(oy + 8, &msg, style(col));
    }

    fn score(&self) -> u32 {
        self.streak
    }

    fn over(&self) -> bool {
        self.lost || (self.streak >= MAX_STREAK && self.reveal.is_none())
    }
}
