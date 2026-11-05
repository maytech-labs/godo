// SPDX-License-Identifier: LicenseRef-REAL-1.0

mod app;
mod art;
mod canvas;
mod games;
mod pet;
mod save;
mod ui;

use std::process::ExitCode;

use crate::art::Expr;
use crate::pet::{Food, Pet, Stage, now_secs};
use crate::save::Store;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
termigatchi — a tiny pet that lives in your terminal

usage:
  termigatchi                open your pet's home (feed, play, clean, …)
  termigatchi status         one line about your pet, for your shell prompt
  termigatchi feed [food]    feed it without opening the app (rice, cookie, cake)
  termigatchi clean          clean up after it
  termigatchi hug            give it a hug
  termigatchi rename <name>  give it a new name
  termigatchi where          print where your pet is saved
  termigatchi reset --yes    say goodbye and start over with a new egg
  termigatchi --version

Your pet keeps living while the app is closed. Set TERMIGATCHI_HOME to keep
it somewhere else.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("termigatchi: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[&str]) -> Result<(), String> {
    let store = Store::locate().map_err(|e| e.to_string())?;
    match args {
        [] => tui(store),
        ["-V" | "--version" | "version"] => {
            println!("termigatchi {VERSION}");
            Ok(())
        }
        ["-h" | "--help" | "help"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["status"] => {
            // Quiet when there's no pet, so it's safe in a prompt.
            if let Some(mut pet) = store.load().map_err(|e| e.to_string())? {
                pet.simulate(now_secs());
                println!("{}", status_line(&pet, now_secs()));
            }
            Ok(())
        }
        ["feed"] => quick(&store, |p, now| p.feed(Food::RiceBall, now)),
        ["feed", food] => {
            let food = Food::parse(food)
                .ok_or_else(|| format!("no food called {food:?}; try rice, cookie, or cake"))?;
            quick(&store, |p, now| p.feed(food, now))
        }
        ["clean"] => quick(&store, |p, now| p.clean(now)),
        ["hug" | "pet"] => quick(&store, |p, now| p.hug(now)),
        ["rename", name @ ..] if !name.is_empty() => {
            let name = name.join(" ");
            let name = name.trim();
            if name.is_empty() || name.chars().count() > app::NAME_MAX {
                return Err(format!("a name needs 1–{} characters", app::NAME_MAX));
            }
            quick(&store, |p, _| {
                let old = std::mem::replace(&mut p.name, name.to_string());
                Ok(format!("{old} is now called {name}."))
            })
        }
        ["where"] => {
            println!("{}", store.path().display());
            Ok(())
        }
        ["reset", "--yes"] => {
            if let Some(pet) = store.load().ok().flatten() {
                println!("Goodbye, {}. ♥", pet.name);
            }
            let _ = std::fs::remove_file(store.path());
            println!("Run `termigatchi` to find a new egg.");
            Ok(())
        }
        ["reset"] => Err(
            "this says goodbye to your pet forever; run `termigatchi reset --yes` if you're sure"
                .into(),
        ),
        _ => Err(format!("unknown command: {}\n\n{USAGE}", args.join(" "))),
    }
}

fn tui(store: Store) -> Result<(), String> {
    let pet = store.load().map_err(|e| e.to_string())?;
    let app = app::App::new(pet, store);
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result.map_err(|e| e.to_string())
}

/// Runs one action on the saved pet, saves, and reports.
fn quick(
    store: &Store,
    f: impl FnOnce(&mut Pet, u64) -> Result<String, String>,
) -> Result<(), String> {
    let mut pet = store
        .load()
        .map_err(|e| e.to_string())?
        .ok_or("you don't have a pet yet; run `termigatchi` to find an egg")?;
    let now = now_secs();
    pet.simulate(now);
    let msg = f(&mut pet, now)?;
    store.save(&pet).map_err(|e| e.to_string())?;
    println!("{msg}");
    println!("{}", status_line(&pet, now));
    Ok(())
}

/// e.g. `(^w^) Mochi ♥♥♥♥♡` or `(o O o) Mochi ♥♥♡♡♡ hungry, poop`.
fn status_line(pet: &Pet, now: u64) -> String {
    if pet.stage(now) == Stage::Egg {
        return format!("(  ) {} is an egg", pet.name);
    }
    let face = art::mini(Expr::from(pet.mood()));
    let full = ((pet.wellbeing() / 20.0).round() as usize).clamp(1, 5);
    let hearts: String = "♥".repeat(full) + &"♡".repeat(5 - full);
    let needs = pet.needs();
    let mut line = format!("{face} {} {hearts}", pet.name);
    if pet.asleep {
        line.push_str(" zZ");
    }
    if !needs.is_empty() {
        line.push(' ');
        line.push_str(&needs.join(", "));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_line_reports_needs() {
        let mut pet = Pet::new("Mochi", 0);
        let now = pet::DAY;
        assert!(status_line(&pet, now).starts_with("(^w^) Mochi ♥♥♥♥"));
        pet.fullness = 10.0;
        pet.poops = 1;
        assert!(status_line(&pet, now).ends_with("hungry, poop"));
        assert_eq!(status_line(&pet, 1), "(  ) Mochi is an egg");
    }
}
