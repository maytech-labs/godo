// SPDX-License-Identifier: LicenseRef-REAL-1.0

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use ratatui::crossterm::event::KeyCode;

use super::{BAD, DIM, GOLD, GOOD, Game, PET, origin, style};
use crate::canvas::Canvas;

const W: i32 = 44;
const H: i32 = 10;
const PET_X: i32 = 4;
const JUMP: f32 = 22.0;
const GRAVITY: f32 = 70.0;

struct Cactus {
    x: f32,
    height: u8,
}

/// An endless runner: hop over the cacti.
pub struct Hop {
    rng: StdRng,
    face: String,
    y: f32,
    vy: f32,
    speed: f32,
    dist: f32,
    cacti: Vec<Cactus>,
    next_gap: f32,
    dead: bool,
    t: f32,
}

impl Hop {
    pub fn new(face: String, seed: u64) -> Hop {
        Hop {
            rng: StdRng::seed_from_u64(seed),
            face,
            y: 0.0,
            vy: 0.0,
            speed: 16.0,
            dist: 0.0,
            cacti: vec![Cactus {
                x: W as f32 + 4.0,
                height: 1,
            }],
            next_gap: 18.0,
            dead: false,
            t: 0.0,
        }
    }

    fn pet_width(&self) -> i32 {
        self.face.chars().count() as i32
    }
}

impl Game for Hop {
    fn tick(&mut self, dt: f32) {
        if self.dead {
            return;
        }
        self.t += dt;
        self.speed = (self.speed + 0.6 * dt).min(45.0);
        self.dist += self.speed * dt;

        if self.y > 0.0 || self.vy > 0.0 {
            self.y += self.vy * dt;
            self.vy -= GRAVITY * dt;
            if self.y <= 0.0 {
                self.y = 0.0;
                self.vy = 0.0;
            }
        }

        // A forgiving hitbox: the middle of the face. Checks the whole span a
        // cactus swept this tick, so a fast one can't pass through.
        let (left, right) = (PET_X + 1, PET_X + self.pet_width() - 1);
        for c in &mut self.cacti {
            let from = c.x.round() as i32;
            c.x -= self.speed * dt;
            let to = c.x.round() as i32;
            if to < right && from >= left && self.y < c.height as f32 {
                self.dead = true;
            }
        }
        self.cacti.retain(|c| c.x > -3.0);
        if self
            .cacti
            .last()
            .is_none_or(|c| c.x < W as f32 - self.next_gap)
        {
            let height = if self.rng.random_bool(0.35) { 2 } else { 1 };
            self.cacti.push(Cactus {
                x: W as f32 + 1.0,
                height,
            });
            self.next_gap = self.rng.random_range(14.0..28.0);
        }
    }

    fn key(&mut self, code: KeyCode) {
        let jump = matches!(
            code,
            KeyCode::Char(' ') | KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('k')
        );
        if jump && self.y == 0.0 && !self.dead {
            self.vy = JUMP;
            self.y = 0.01;
        }
    }

    fn draw(&self, c: &mut Canvas) {
        let (ox, oy) = origin(c, W, H + 1);
        let s = format!("{}", self.score());
        c.text(ox + W - s.len() as i32, oy, &s, style(GOLD));
        // Drifting clouds.
        for (i, base) in [(0, 6.0), (1, 30.0)] {
            let x = (base - self.dist * 0.15).rem_euclid(W as f32 + 8.0) as i32 - 4;
            c.text(ox + x, oy + 1 + i, "~~", style(DIM));
        }
        let ground = oy + H;
        for x in 0..W {
            let ch = if (x + self.dist as i32) % 7 == 0 {
                '.'
            } else {
                '_'
            };
            c.put(ox + x, ground, ch, style(DIM));
        }
        for cactus in &self.cacti {
            let x = ox + cactus.x.round() as i32;
            if x < ox || x >= ox + W {
                continue;
            }
            c.put(x, ground - 1, '|', style(GOOD));
            if cactus.height == 2 {
                c.put(x, ground - 2, 'Y', style(GOOD));
            }
        }
        let top = ground - 2 - self.y.round() as i32;
        let face = if self.dead {
            "(x_x)".to_string()
        } else {
            self.face.clone()
        };
        c.text(
            ox + PET_X,
            top,
            &face,
            style(if self.dead { BAD } else { PET }),
        );
        let feet = if self.y > 0.0 {
            " ^ ^ "
        } else if (self.t * 8.0) as i32 % 2 == 0 {
            " d b "
        } else {
            " b d "
        };
        c.text(ox + PET_X, top + 1, feet, style(PET));
        if self.dead {
            c.text_centered(oy + H / 2, " Bonk! ", style(GOLD));
        }
    }

    fn score(&self) -> u32 {
        (self.dist / 4.0) as u32
    }

    fn over(&self) -> bool {
        self.dead
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jumping_clears_a_cactus() {
        let mut g = Hop::new("(^w^)".into(), 1);
        g.cacti = vec![Cactus { x: 12.0, height: 2 }];
        g.next_gap = 1000.0;
        // Jump when the cactus is a few columns away, then let it pass.
        for _ in 0..200 {
            if g.y == 0.0 && g.cacti.iter().any(|c| c.x > 6.0 && c.x < 10.0) {
                g.key(KeyCode::Char(' '));
            }
            g.tick(0.016);
        }
        assert!(!g.dead);
    }

    #[test]
    fn standing_still_bonks() {
        let mut g = Hop::new("(^w^)".into(), 1);
        for _ in 0..400 {
            g.tick(0.016);
        }
        assert!(g.dead);
    }
}
