// SPDX-License-Identifier: LicenseRef-REAL-1.0

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use ratatui::crossterm::event::KeyCode;

use super::{BAD, DIM, GOLD, GOOD, Game, PET, origin, style};
use crate::canvas::Canvas;

const W: i32 = 26;
const H: i32 = 14;
const DURATION: f32 = 30.0;
const SNACKS: [char; 4] = ['o', '@', '%', '&'];

struct Item {
    x: i32,
    y: f32,
    good: bool,
    ch: char,
}

/// Snacks and rocks fall; catch the snacks.
pub struct SnackRain {
    rng: StdRng,
    face: String,
    x: i32,
    items: Vec<Item>,
    lives: u32,
    score: u32,
    time: f32,
    spawn: f32,
    flash: f32,
}

impl SnackRain {
    pub fn new(face: String, seed: u64) -> SnackRain {
        SnackRain {
            rng: StdRng::seed_from_u64(seed),
            face,
            x: W / 2 - 1,
            items: Vec::new(),
            lives: 3,
            score: 0,
            time: DURATION,
            spawn: 0.5,
            flash: 0.0,
        }
    }

    fn elapsed(&self) -> f32 {
        DURATION - self.time
    }
}

impl Game for SnackRain {
    fn tick(&mut self, dt: f32) {
        if self.over() {
            return;
        }
        self.time = (self.time - dt).max(0.0);
        self.flash = (self.flash - dt).max(0.0);
        let speed = 5.0 + self.elapsed() * 0.2;

        self.spawn -= dt;
        if self.spawn <= 0.0 {
            self.spawn = (0.8 - self.elapsed() * 0.015).max(0.3) + self.rng.random_range(0.0..0.3);
            let good = self.rng.random_bool(0.72);
            let ch = if good {
                SNACKS[self.rng.random_range(0..SNACKS.len())]
            } else {
                '#'
            };
            self.items.push(Item {
                x: self.rng.random_range(0..W),
                y: 0.0,
                good,
                ch,
            });
        }

        let catch_row = (H - 1) as f32;
        let (bx, mut caught, mut hit) = (self.x, 0, 0);
        self.items.retain_mut(|it| {
            let before = it.y;
            it.y += speed * dt;
            if before < catch_row && it.y >= catch_row && (bx..bx + 3).contains(&it.x) {
                if it.good {
                    caught += 1
                } else {
                    hit += 1
                }
                return false;
            }
            it.y < H as f32
        });
        self.score += caught;
        if hit > 0 {
            self.lives = self.lives.saturating_sub(hit);
            self.flash = 0.4;
        }
    }

    fn key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('h') => self.x = (self.x - 2).max(0),
            KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('l') => {
                self.x = (self.x + 2).min(W - 3)
            }
            _ => {}
        }
    }

    fn draw(&self, c: &mut Canvas) {
        let (ox, oy) = origin(c, W + 2, H + 3);
        let hearts: String = (0..3)
            .map(|i| if i < self.lives { '♥' } else { '·' })
            .collect();
        c.text(ox, oy, &hearts, style(BAD));
        c.text_centered(oy, &format!("{}", self.score), style(GOLD));
        let t = format!("{:>2}s", self.time.ceil() as u32);
        c.text(ox + W + 2 - t.len() as i32, oy, &t, style(DIM));

        let wall = style(if self.flash > 0.0 { BAD } else { DIM });
        for y in 0..H {
            c.put(ox, oy + 1 + y, '│', wall);
            c.put(ox + W + 1, oy + 1 + y, '│', wall);
        }
        for it in &self.items {
            c.put(
                ox + 1 + it.x,
                oy + 1 + it.y as i32,
                it.ch,
                style(if it.good { GOOD } else { DIM }),
            );
        }
        c.text(ox + 1 + self.x, oy + H, "\\_/", style(GOLD));
        let fw = self.face.chars().count() as i32;
        c.text(
            ox + 1 + self.x + 1 - fw / 2,
            oy + H + 1,
            &self.face,
            style(PET),
        );
        if self.over() {
            c.text_centered(
                oy + H / 2,
                if self.lives == 0 {
                    " Ouch! "
                } else {
                    " Time! "
                },
                style(GOLD),
            );
        }
    }

    fn score(&self) -> u32 {
        self.score
    }

    fn over(&self) -> bool {
        self.lives == 0 || self.time <= 0.0
    }
}
