// SPDX-License-Identifier: LicenseRef-REAL-1.0

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use ratatui::crossterm::event::KeyCode;

use super::{BAD, DIM, GOLD, GOOD, Game, PET, origin, style};
use crate::canvas::Canvas;

const ROUNDS: u32 = 5;

/// The classic: guess which way your pet will turn.
pub struct WhichWay {
    rng: StdRng,
    face: String,
    round: u32,
    score: u32,
    /// (guessed left, looked left, seconds left to show it)
    reveal: Option<(bool, bool, f32)>,
    t: f32,
}

impl WhichWay {
    pub fn new(face: String, seed: u64) -> WhichWay {
        WhichWay {
            rng: StdRng::seed_from_u64(seed),
            face,
            round: 0,
            score: 0,
            reveal: None,
            t: 0.0,
        }
    }
}

impl Game for WhichWay {
    fn tick(&mut self, dt: f32) {
        self.t += dt;
        if let Some((_, _, left)) = &mut self.reveal {
            *left -= dt;
            if *left <= 0.0 {
                self.reveal = None;
                self.round += 1;
            }
        }
    }

    fn key(&mut self, code: KeyCode) {
        if self.reveal.is_some() || self.over() {
            return;
        }
        let guess_left = match code {
            KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('h') => true,
            KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('l') => false,
            _ => return,
        };
        let looked_left = self.rng.random_bool(0.5);
        if guess_left == looked_left {
            self.score += 1;
        }
        self.reveal = Some((guess_left, looked_left, 1.2));
    }

    fn draw(&self, c: &mut Canvas) {
        let (ox, oy) = origin(c, 30, 9);
        let mid = ox + 15;
        let shown = (self.round + 1).min(ROUNDS);
        c.text(ox, oy, &format!("Round {shown}/{ROUNDS}"), style(DIM));
        let s = format!("Score {}", self.score);
        c.text(ox + 30 - s.len() as i32, oy, &s, style(GOLD));

        let w = self.face.chars().count() as i32;
        match self.reveal {
            None => {
                // Fidget while waiting for a guess.
                let peek = ((self.t * 3.0) as i32 % 4) - 1;
                let dx = peek.clamp(-1, 1);
                c.text(mid - w / 2 + dx, oy + 4, &self.face, style(PET));
                c.text_centered(oy + 2, "?", style(GOLD));
                c.text(ox, oy + 4, "←", style(DIM));
                c.text(ox + 29, oy + 4, "→", style(DIM));
                c.text_centered(oy + 7, "Which way will I look?", style(DIM));
            }
            Some((guess, looked, _)) => {
                let (x, arrow) = if looked {
                    (mid - w / 2 - 6, "<")
                } else {
                    (mid - w / 2 + 6, ">")
                };
                c.text(x, oy + 4, &self.face, style(PET));
                c.text(
                    if looked { x - 2 } else { x + w + 1 },
                    oy + 4,
                    arrow,
                    style(PET),
                );
                let (msg, col) = if guess == looked {
                    ("Yes! You got it!", GOOD)
                } else {
                    ("Nope!", BAD)
                };
                c.text_centered(oy + 7, msg, style(col));
            }
        }
    }

    fn score(&self) -> u32 {
        self.score
    }

    fn over(&self) -> bool {
        self.round >= ROUNDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guessing_is_a_coin_flip() {
        let total: u32 = (0..200)
            .map(|seed| {
                let mut g = WhichWay::new("(^w^)".into(), seed);
                while !g.over() {
                    g.key(KeyCode::Left);
                    g.tick(2.0);
                }
                g.score()
            })
            .sum();
        // 1000 guesses; expect ~500.
        assert!((400..600).contains(&total), "{total}");
    }
}
