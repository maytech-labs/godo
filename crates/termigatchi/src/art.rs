// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! The pet's look: one sprite per life stage, with eyes and a mouth that
//! change with its expression.

use crate::pet::{Mood, Stage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expr {
    Happy,
    Content,
    Hungry,
    Sleepy,
    Sad,
    Dirty,
    Sick,
    Sleeping,
    Blink,
    Eating(bool),
    Love,
    Pout,
    Cheer,
}

impl From<Mood> for Expr {
    fn from(m: Mood) -> Expr {
        match m {
            Mood::Happy => Expr::Happy,
            Mood::Content => Expr::Content,
            Mood::Hungry => Expr::Hungry,
            Mood::Sleepy => Expr::Sleepy,
            Mood::Sad => Expr::Sad,
            Mood::Dirty => Expr::Dirty,
            Mood::Sick => Expr::Sick,
            Mood::Sleeping => Expr::Sleeping,
        }
    }
}

/// Left eye, right eye, mouth.
pub fn parts(e: Expr) -> (char, char, char) {
    match e {
        Expr::Happy => ('^', '^', 'w'),
        Expr::Content => ('o', 'o', 'v'),
        Expr::Hungry => ('o', 'o', 'O'),
        Expr::Sleepy => ('=', '=', 'o'),
        Expr::Sad => ('T', 'T', 'n'),
        Expr::Dirty => ('>', '<', '~'),
        Expr::Sick => ('x', 'x', '~'),
        Expr::Sleeping => ('-', '-', '.'),
        Expr::Blink => ('-', '-', 'v'),
        Expr::Eating(open) => ('^', '^', if open { 'O' } else { 'o' }),
        Expr::Love => ('♥', '♥', 'w'),
        Expr::Pout => ('>', '<', '^'),
        Expr::Cheer => ('*', '*', 'D'),
    }
}

const EGG: [&str; 5] = [
    "   __   ",
    "  /  \\  ",
    " / .  \\ ",
    "|  . . |",
    " \\____/ ",
];

const EGG_CRACKED: [&str; 5] = [
    "   __   ",
    "  /  \\  ",
    " / .  \\ ",
    "| /\\/\\ |",
    " \\____/ ",
];

const BABY: [&str; 4] = ["   .---.   ", "  / L R \\  ", " (   M   ) ", "  `-----'  "];

const KID: [&str; 7] = [
    "   .-\"\"\"-.   ",
    "  /       \\  ",
    " |  L   R  | ",
    " | .  M  . | ",
    "  \\       /  ",
    "   '-...-'   ",
    "    ^   ^    ",
];

const TEEN: [&str; 9] = [
    "      ,_     ",
    "      |/     ",
    "   .-\"\"\"-.   ",
    "  /       \\  ",
    " |  L   R  | ",
    " | .  M  . | ",
    "  \\       /  ",
    "   '-...-'   ",
    "    d   b    ",
];

const ADULT: [&str; 9] = [
    "       (@)       ",
    "        |/       ",
    "    .-'\"\"\"'-.    ",
    "   /         \\   ",
    "  |   L   R   |  ",
    " o|  .  M  .  |o ",
    "   \\         /   ",
    "    '-.___.-'    ",
    "      d   b      ",
];

/// The pet drawn at `stage` with expression `e`. `cracked` only matters for
/// eggs, which crack as they get close to hatching.
pub fn sprite(stage: Stage, e: Expr, cracked: bool) -> Vec<String> {
    let template: &[&str] = match stage {
        Stage::Egg if cracked => &EGG_CRACKED,
        Stage::Egg => &EGG,
        Stage::Baby => &BABY,
        Stage::Kid => &KID,
        Stage::Teen => &TEEN,
        Stage::Adult => &ADULT,
    };
    let (l, r, m) = parts(e);
    template
        .iter()
        .map(|line| {
            line.chars()
                .map(|c| match c {
                    'L' => l,
                    'R' => r,
                    'M' => m,
                    c => c,
                })
                .collect()
        })
        .collect()
}

/// A one-line face, for games and the shell prompt.
pub fn mini(e: Expr) -> String {
    let (l, r, m) = parts(e);
    format!("({l}{m}{r})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprites_are_rectangular() {
        for stage in [
            Stage::Egg,
            Stage::Baby,
            Stage::Kid,
            Stage::Teen,
            Stage::Adult,
        ] {
            for cracked in [false, true] {
                let s = sprite(stage, Expr::Happy, cracked);
                let w = s[0].chars().count();
                assert!(s.iter().all(|l| l.chars().count() == w), "{stage:?}");
            }
        }
    }

    #[test]
    fn faces_change_with_expression() {
        assert_eq!(mini(Expr::Happy), "(^w^)");
        assert_eq!(mini(Expr::Sick), "(x~x)");
        assert!(sprite(Stage::Kid, Expr::Sad, false)[2].contains('T'));
    }
}
