// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! The pet and its life: needs that drift with real time, and the things
//! you can do for it. Everything here is deterministic, so `termigatchi
//! status` and the TUI always agree about how the pet is doing.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const HOUR: u64 = 3600;
pub const DAY: u64 = 24 * HOUR;
/// How long a new egg takes to hatch.
pub const EGG_SECS: u64 = 30;
/// Time away beyond this is forgiven, not simulated.
const MAX_CATCHUP: u64 = 30 * DAY;
const HUG_COOLDOWN: u64 = 8;
const MAX_POOPS: u8 = 4;

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Egg,
    Baby,
    Kid,
    Teen,
    Adult,
}

impl Stage {
    pub fn at_age(age: u64) -> Stage {
        match age {
            a if a < EGG_SECS => Stage::Egg,
            a if a < DAY => Stage::Baby,
            a if a < 3 * DAY => Stage::Kid,
            a if a < 7 * DAY => Stage::Teen,
            _ => Stage::Adult,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Stage::Egg => "Egg",
            Stage::Baby => "Baby",
            Stage::Kid => "Kid",
            Stage::Teen => "Teen",
            Stage::Adult => "Adult",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Sleeping,
    Sick,
    Hungry,
    Sleepy,
    Dirty,
    Sad,
    Happy,
    Content,
}

impl Mood {
    pub fn name(self) -> &'static str {
        match self {
            Mood::Sleeping => "sleeping",
            Mood::Sick => "sick",
            Mood::Hungry => "hungry",
            Mood::Sleepy => "sleepy",
            Mood::Dirty => "icky",
            Mood::Sad => "lonely",
            Mood::Happy => "happy",
            Mood::Content => "content",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Food {
    RiceBall,
    Cookie,
    Cake,
}

impl Food {
    pub const ALL: [Food; 3] = [Food::RiceBall, Food::Cookie, Food::Cake];

    pub fn name(self) -> &'static str {
        match self {
            Food::RiceBall => "Rice ball",
            Food::Cookie => "Cookie",
            Food::Cake => "Cake",
        }
    }

    pub fn cost(self) -> u32 {
        match self {
            Food::RiceBall => 0,
            Food::Cookie => 5,
            Food::Cake => 15,
        }
    }

    fn fullness(self) -> f32 {
        match self {
            Food::RiceBall => 30.0,
            Food::Cookie => 10.0,
            Food::Cake => 8.0,
        }
    }

    fn fun(self) -> f32 {
        match self {
            Food::RiceBall => 2.0,
            Food::Cookie => 15.0,
            Food::Cake => 35.0,
        }
    }

    pub fn blurb(self) -> String {
        let price = match self.cost() {
            0 => "free".to_string(),
            c => format!("¢{c}"),
        };
        format!(
            "{price:>4}  +{} food  +{} fun",
            self.fullness() as u32,
            self.fun() as u32
        )
    }

    pub fn parse(s: &str) -> Option<Food> {
        match s.to_ascii_lowercase().as_str() {
            "rice" | "riceball" | "rice-ball" | "meal" => Some(Food::RiceBall),
            "cookie" | "snack" => Some(Food::Cookie),
            "cake" => Some(Food::Cake),
            _ => None,
        }
    }
}

/// What the pet's needs did over one `simulate` call, for the UI to narrate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub fell_asleep: bool,
    pub woke_up: bool,
    pub new_poops: u8,
    pub got_sick: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pet {
    pub name: String,
    /// Unix seconds.
    pub born: u64,
    /// Unix seconds the needs were last brought up to date.
    pub updated: u64,
    pub fullness: f32,
    pub happiness: f32,
    pub energy: f32,
    pub hygiene: f32,
    pub health: f32,
    pub asleep: bool,
    pub poops: u8,
    poop_meter: f32,
    pub coins: u32,
    pub xp: u32,
    pub best: BTreeMap<String, u32>,
    pub games_played: u32,
    pub meals: u32,
    last_hug: u64,
}

impl Default for Pet {
    fn default() -> Self {
        Pet::new("Mochi", 0)
    }
}

fn clamp(v: &mut f32) {
    *v = v.clamp(0.0, 100.0);
}

impl Pet {
    pub fn new(name: &str, now: u64) -> Pet {
        Pet {
            name: name.to_string(),
            born: now,
            updated: now,
            fullness: 70.0,
            happiness: 70.0,
            energy: 80.0,
            hygiene: 100.0,
            health: 100.0,
            asleep: false,
            poops: 0,
            poop_meter: 0.0,
            coins: 10,
            xp: 0,
            best: BTreeMap::new(),
            games_played: 0,
            meals: 0,
            last_hug: 0,
        }
    }

    pub fn age(&self, now: u64) -> u64 {
        now.saturating_sub(self.born)
    }

    pub fn stage(&self, now: u64) -> Stage {
        Stage::at_age(self.age(now))
    }

    pub fn sick(&self) -> bool {
        self.health < 40.0
    }

    pub fn level(&self) -> u32 {
        (self.xp as f32 / 12.0).sqrt() as u32 + 1
    }

    /// Average of all needs, 0–100.
    pub fn wellbeing(&self) -> f32 {
        (self.fullness + self.happiness + self.energy + self.hygiene + self.health) / 5.0
    }

    pub fn mood(&self) -> Mood {
        if self.asleep {
            Mood::Sleeping
        } else if self.sick() {
            Mood::Sick
        } else if self.fullness < 25.0 {
            Mood::Hungry
        } else if self.energy < 20.0 {
            Mood::Sleepy
        } else if self.poops >= 2 || self.hygiene < 25.0 {
            Mood::Dirty
        } else if self.happiness < 30.0 {
            Mood::Sad
        } else if self.happiness >= 70.0 && self.wellbeing() >= 60.0 {
            Mood::Happy
        } else {
            Mood::Content
        }
    }

    /// The things the pet wants right now, most urgent first.
    pub fn needs(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.sick() {
            out.push("sick");
        }
        if self.fullness < 25.0 {
            out.push("hungry");
        }
        if self.poops > 0 {
            out.push("poop");
        }
        if !self.asleep && self.energy < 20.0 {
            out.push("sleepy");
        }
        if self.happiness < 30.0 {
            out.push("bored");
        }
        out
    }

    /// Brings the needs up to `now`, a minute at a time.
    pub fn simulate(&mut self, now: u64) -> Changes {
        let mut changes = Changes::default();
        if now <= self.updated {
            return changes;
        }
        let mut t = self.updated.max(now.saturating_sub(MAX_CATCHUP));
        // Eggs don't get hungry.
        t = t.max(self.born + EGG_SECS).min(now);
        let was_sick = self.sick();
        while t < now {
            let step = (now - t).min(60);
            self.step(step as f32 / HOUR as f32, &mut changes);
            t += step;
        }
        changes.got_sick = !was_sick && self.sick();
        self.updated = now;
        changes
    }

    fn step(&mut self, h: f32, changes: &mut Changes) {
        let awake = !self.asleep;
        self.fullness -= if awake { 6.0 } else { 3.0 } * h;
        self.energy += if awake { -5.0 } else { 16.0 } * h;
        self.hygiene -= 3.0 * h;

        if awake {
            self.poop_meter += h / 3.0;
            if self.poop_meter >= 1.0 {
                self.poop_meter -= 1.0;
                if self.poops < MAX_POOPS {
                    self.poops += 1;
                    self.hygiene -= 10.0;
                    changes.new_poops += 1;
                }
            }
        }

        let mut gloom = 4.0 + 2.0 * self.poops as f32;
        if self.sick() {
            gloom += 3.0;
        }
        if self.fullness < 20.0 {
            gloom += 3.0;
        }
        if self.asleep {
            gloom *= 0.25;
        }
        self.happiness -= gloom * h;

        let neglected = self.fullness < 15.0 || self.hygiene < 15.0 || self.poops >= 3;
        if neglected {
            self.health -= 4.0 * h;
        } else if self.fullness > 50.0 && self.hygiene > 50.0 {
            self.health += 2.0 * h;
        }

        self.clamp_all();

        if awake && self.energy <= 0.0 {
            self.asleep = true;
            changes.fell_asleep = true;
        } else if self.asleep && self.energy >= 100.0 {
            self.asleep = false;
            changes.woke_up = true;
        }
    }

    fn clamp_all(&mut self) {
        clamp(&mut self.fullness);
        clamp(&mut self.happiness);
        clamp(&mut self.energy);
        clamp(&mut self.hygiene);
        clamp(&mut self.health);
    }

    fn ready(&self, now: u64) -> Result<(), String> {
        if self.stage(now) == Stage::Egg {
            return Err("It's still an egg… give it a moment to hatch!".into());
        }
        Ok(())
    }

    fn awake(&self, now: u64) -> Result<(), String> {
        self.ready(now)?;
        if self.asleep {
            return Err(format!("Shh… {} is sleeping. zZ", self.name));
        }
        Ok(())
    }

    pub fn feed(&mut self, food: Food, now: u64) -> Result<String, String> {
        self.awake(now)?;
        if food == Food::RiceBall && self.fullness >= 95.0 {
            return Err(format!("{} is stuffed! Maybe later.", self.name));
        }
        if self.fullness >= 100.0 {
            return Err(format!("{} couldn't eat another bite.", self.name));
        }
        if self.coins < food.cost() {
            return Err(format!(
                "A {} costs ¢{} — win some coins in a game first!",
                food.name().to_lowercase(),
                food.cost()
            ));
        }
        self.coins -= food.cost();
        self.fullness += food.fullness();
        self.happiness += food.fun();
        self.meals += 1;
        self.xp += 1;
        self.clamp_all();
        Ok(match food {
            Food::RiceBall => format!("{} munches the rice ball. Nom nom!", self.name),
            Food::Cookie => format!("{} crunches the cookie happily!", self.name),
            Food::Cake => format!("CAKE! {} is over the moon!", self.name),
        })
    }

    pub fn clean(&mut self, now: u64) -> Result<String, String> {
        self.ready(now)?;
        if self.poops == 0 && self.hygiene >= 90.0 {
            return Err("Everything's already sparkly!".into());
        }
        let had = self.poops;
        self.poops = 0;
        self.hygiene = 100.0;
        self.happiness += 5.0;
        self.xp += 1;
        self.clamp_all();
        Ok(if had > 0 {
            format!(
                "Scooped {had} poop{}. Fresh!",
                if had == 1 { "" } else { "s" }
            )
        } else {
            format!("Bath time! {} is squeaky clean.", self.name)
        })
    }

    pub fn toggle_sleep(&mut self, now: u64) -> Result<String, String> {
        self.ready(now)?;
        if self.asleep {
            self.asleep = false;
            if self.energy < 50.0 {
                self.happiness -= 5.0;
                self.clamp_all();
                return Ok(format!("{} wakes up grumpy… still tired.", self.name));
            }
            return Ok(format!("Good morning, {}!", self.name));
        }
        if self.energy >= 90.0 {
            return Err(format!("{} isn't sleepy at all!", self.name));
        }
        self.asleep = true;
        Ok(format!("Lights out. Sweet dreams, {}.", self.name))
    }

    pub fn medicine(&mut self, now: u64) -> Result<String, String> {
        self.awake(now)?;
        if !self.sick() {
            return Err(format!("{} isn't sick — no medicine needed.", self.name));
        }
        self.health += 45.0;
        self.happiness -= 5.0;
        self.clamp_all();
        Ok(format!("Yuck! But {} feels better already.", self.name))
    }

    pub fn hug(&mut self, now: u64) -> Result<String, String> {
        self.awake(now)?;
        if now.saturating_sub(self.last_hug) < HUG_COOLDOWN {
            return Err(format!("{} wriggles — a little space, please!", self.name));
        }
        self.last_hug = now;
        self.happiness += 6.0;
        self.xp += 1;
        self.clamp_all();
        Ok(format!("{} snuggles up to you. ♥", self.name))
    }

    pub fn can_play(&self, now: u64) -> Result<(), String> {
        self.awake(now)?;
        if self.sick() {
            return Err(format!(
                "{} is too sick to play. Medicine first!",
                self.name
            ));
        }
        if self.energy < 10.0 {
            return Err(format!("{} is too tired to play.", self.name));
        }
        Ok(())
    }

    /// Records a finished game. Returns true for a new best score.
    pub fn finish_game(&mut self, game: &str, score: u32, coins: u32) -> bool {
        self.happiness += 8.0 + (score as f32).min(12.0);
        self.energy -= 8.0;
        self.fullness -= 4.0;
        self.coins += coins;
        self.xp += 3 + score.min(20);
        self.games_played += 1;
        self.clamp_all();
        let best = self.best.entry(game.to_string()).or_insert(0);
        let record = score > *best;
        if record {
            *best = score;
        }
        record
    }

    pub fn best(&self, game: &str) -> u32 {
        self.best.get(game).copied().unwrap_or(0)
    }
}

pub fn fmt_age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < HOUR => format!("{}m", s / 60),
        s if s < DAY => format!("{}h {}m", s / HOUR, (s % HOUR) / 60),
        s => format!("{}d {}h", s / DAY, (s % DAY) / HOUR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;

    fn hatched() -> Pet {
        let mut p = Pet::new("Mochi", T0);
        p.simulate(T0 + EGG_SECS);
        p
    }

    #[test]
    fn stages_follow_age() {
        assert_eq!(Stage::at_age(0), Stage::Egg);
        assert_eq!(Stage::at_age(EGG_SECS), Stage::Baby);
        assert_eq!(Stage::at_age(DAY), Stage::Kid);
        assert_eq!(Stage::at_age(3 * DAY), Stage::Teen);
        assert_eq!(Stage::at_age(30 * DAY), Stage::Adult);
    }

    #[test]
    fn eggs_do_not_change() {
        let mut p = Pet::new("Mochi", T0);
        let before = p.clone();
        p.simulate(T0 + EGG_SECS - 1);
        assert_eq!(p.fullness, before.fullness);
        assert!(p.feed(Food::RiceBall, T0 + 1).is_err());
    }

    #[test]
    fn needs_drift_over_time() {
        let mut p = hatched();
        let t = p.updated;
        p.simulate(t + 2 * HOUR);
        assert!((p.fullness - 58.0).abs() < 0.01, "{}", p.fullness);
        assert!(p.energy < 80.0);
        assert!(p.happiness < 70.0);
    }

    #[test]
    fn simulation_is_split_invariant() {
        let mut a = hatched();
        let mut b = a.clone();
        let t = a.updated;
        a.simulate(t + 10 * HOUR);
        for i in 1..=600 {
            b.simulate(t + i * 60);
        }
        assert!((a.fullness - b.fullness).abs() < 0.01);
        assert_eq!(a.poops, b.poops);
        assert_eq!(a.asleep, b.asleep);
    }

    #[test]
    fn poops_appear_and_cap() {
        let mut p = hatched();
        let t = p.updated;
        p.simulate(t + 3 * HOUR + 60);
        assert_eq!(p.poops, 1);
        p.simulate(t + 3 * DAY);
        assert!(p.poops <= MAX_POOPS);
    }

    #[test]
    fn neglect_makes_sick_but_never_kills() {
        let mut p = hatched();
        let t = p.updated;
        p.simulate(t + 25 * DAY);
        assert!(p.sick());
        assert_eq!(p.mood(), Mood::Sick);
        assert!(p.medicine(p.updated).is_ok() || p.asleep);
    }

    #[test]
    fn runs_out_of_energy_then_sleeps_and_wakes() {
        let mut p = hatched();
        p.energy = 1.0;
        let t = p.updated;
        let c = p.simulate(t + HOUR);
        assert!(c.fell_asleep && p.asleep);
        let c = p.simulate(t + 9 * HOUR);
        assert!(c.woke_up && !p.asleep);
    }

    #[test]
    fn long_absence_is_capped() {
        let mut p = hatched();
        let t = p.updated;
        p.simulate(t + 400 * DAY);
        assert_eq!(p.updated, t + 400 * DAY);
    }

    #[test]
    fn feeding_costs_coins_and_refuses_when_full() {
        let mut p = hatched();
        let now = p.updated;
        p.coins = 4;
        assert!(p.feed(Food::Cookie, now).is_err());
        assert!(p.feed(Food::RiceBall, now).is_ok());
        assert_eq!(p.fullness, 100.0);
        assert!(p.feed(Food::RiceBall, now).is_err());
    }

    #[test]
    fn hugs_have_a_cooldown() {
        let mut p = hatched();
        let now = p.updated;
        assert!(p.hug(now).is_ok());
        assert!(p.hug(now + 1).is_err());
        assert!(p.hug(now + HUG_COOLDOWN).is_ok());
    }

    #[test]
    fn games_pay_out_and_track_bests() {
        let mut p = hatched();
        assert!(p.finish_game("hop", 10, 4));
        assert!(!p.finish_game("hop", 3, 1));
        assert_eq!(p.best("hop"), 10);
        assert_eq!(p.coins, 15);
    }

    #[test]
    fn old_saves_fill_in_defaults() {
        let p: Pet = serde_json::from_str(r#"{"name":"Bean","born":5}"#).unwrap();
        assert_eq!(p.name, "Bean");
        assert_eq!(p.health, 100.0);
    }
}
