// SPDX-License-Identifier: LicenseRef-REAL-1.0

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use ratatui::crossterm::event::KeyCode;
use ratatui::style::{Color, Modifier, Style};

use super::{BAD, DIM, GOLD, Game, PET, origin, style};
use crate::canvas::Canvas;

const PADS: [(&str, Color, (i32, i32)); 4] = [
    (" ↑ ", Color::Rgb(255, 120, 150), (8, 0)),
    (" → ", Color::Rgb(120, 200, 255), (16, 3)),
    (" ↓ ", Color::Rgb(150, 230, 140), (8, 6)),
    (" ← ", Color::Rgb(255, 210, 110), (0, 3)),
];
const ON: f32 = 0.45;
const GAP: f32 = 0.15;

enum Phase {
    /// Showing the pattern: index of the pad shown and time in this step.
    Show(usize, f32),
    /// The player repeats it: how many they've got right so far.
    Input(usize),
    Wrong,
}

/// Watch the pattern, then repeat it. It grows by one each round.
pub struct Echo {
    rng: StdRng,
    face: String,
    seq: Vec<usize>,
    phase: Phase,
    score: u32,
    /// The pad the player last pressed, lit briefly.
    pressed: Option<(usize, f32)>,
}

impl Echo {
    pub fn new(face: String, seed: u64) -> Echo {
        let mut rng = StdRng::seed_from_u64(seed);
        let first = rng.random_range(0..4);
        Echo {
            rng,
            face,
            seq: vec![first],
            phase: Phase::Show(0, -0.8),
            score: 0,
            pressed: None,
        }
    }

    fn lit(&self) -> Option<usize> {
        match self.phase {
            Phase::Show(i, t) if (0.0..ON).contains(&t) => Some(self.seq[i]),
            _ => self.pressed.map(|(p, _)| p),
        }
    }
}

impl Game for Echo {
    fn tick(&mut self, dt: f32) {
        if let Some((_, t)) = &mut self.pressed {
            *t -= dt;
            if *t <= 0.0 {
                self.pressed = None;
            }
        }
        if let Phase::Show(i, t) = &mut self.phase {
            *t += dt;
            if *t >= ON + GAP {
                if *i + 1 < self.seq.len() {
                    *i += 1;
                    *t = 0.0;
                } else {
                    self.phase = Phase::Input(0);
                }
            }
        }
    }

    fn key(&mut self, code: KeyCode) {
        let Phase::Input(got) = self.phase else {
            return;
        };
        let pad = match code {
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('k') => 0,
            KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('l') => 1,
            KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('j') => 2,
            KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('h') => 3,
            _ => return,
        };
        self.pressed = Some((pad, 0.2));
        if self.seq[got] != pad {
            self.phase = Phase::Wrong;
        } else if got + 1 == self.seq.len() {
            self.score = self.seq.len() as u32;
            self.seq.push(self.rng.random_range(0..4));
            self.phase = Phase::Show(0, -0.8);
        } else {
            self.phase = Phase::Input(got + 1);
        }
    }

    fn draw(&self, c: &mut Canvas) {
        let (ox, oy) = origin(c, 21, 11);
        let lit = self.lit();
        for (i, (label, color, (dx, dy))) in PADS.iter().enumerate() {
            let s = if lit == Some(i) {
                Style::default()
                    .fg(Color::Black)
                    .bg(*color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(*color)
            };
            c.text(ox + dx, oy + dy, "┌───┐", style(*color));
            c.text(ox + dx, oy + dy + 1, "│   │", style(*color));
            c.text(ox + dx + 1, oy + dy + 1, label, s);
            c.text(ox + dx, oy + dy + 2, "└───┘", style(*color));
        }
        let fw = self.face.chars().count() as i32;
        let face = if matches!(self.phase, Phase::Wrong) {
            "(>_<)"
        } else {
            &self.face
        };
        c.text(ox + 10 - fw / 2, oy + 4, face, style(PET));

        let (msg, col) = match self.phase {
            Phase::Show(..) => ("Watch…".to_string(), DIM),
            Phase::Input(got) => (format!("Your turn! {got}/{}", self.seq.len()), GOLD),
            Phase::Wrong => ("Oops! That wasn't it.".to_string(), BAD),
        };
        c.text_centered(oy + 10, &msg, style(col));
        c.text_centered(oy - 1, &format!("Round {}", self.score + 1), style(DIM));
    }

    fn score(&self) -> u32 {
        self.score
    }

    fn over(&self) -> bool {
        matches!(self.phase, Phase::Wrong)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [KeyCode; 4] = [KeyCode::Up, KeyCode::Right, KeyCode::Down, KeyCode::Left];

    #[test]
    fn repeating_the_pattern_scores() {
        let mut g = Echo::new("(^w^)".into(), 3);
        for round in 1..=4 {
            while !matches!(g.phase, Phase::Input(_)) {
                g.tick(0.05);
            }
            for p in g.seq.clone() {
                g.key(KEYS[p]);
            }
            assert_eq!(g.score, round);
        }
        while !matches!(g.phase, Phase::Input(_)) {
            g.tick(0.05);
        }
        g.key(KEYS[(g.seq[0] + 1) % 4]);
        assert!(g.over());
        assert_eq!(g.score(), 4);
    }
}
