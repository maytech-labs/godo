// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! The interactive app: screens, keys, and the little animations that make
//! the pet feel alive.

use std::io;
use std::time::{Duration, Instant};

use rand::Rng;
use rand::rngs::ThreadRng;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Color;

use crate::art::{self, Expr};
use crate::games::{Game, GameKind};
use crate::pet::{Food, Mood, Pet, Stage, now_secs};
use crate::save::Store;
use crate::ui;

pub const NAME_MAX: usize = 16;

pub enum Screen {
    Home,
    Feed(usize),
    Play(usize),
    Game {
        kind: GameKind,
        game: Box<dyn Game>,
        /// App time at which to leave for the results, once it's over.
        ended: Option<f32>,
    },
    Results {
        kind: GameKind,
        score: u32,
        coins: u32,
        record: bool,
    },
    Rename {
        input: String,
        first: bool,
    },
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    Eat,
    Love,
    Sparkle,
    Nope,
    Cheer,
}

/// A floating bit of decoration, positioned relative to the top-center of
/// the pet's sprite.
pub struct Particle {
    pub x: f32,
    pub y: f32,
    vx: f32,
    vy: f32,
    pub ch: char,
    pub color: Color,
    life: f32,
}

pub struct App {
    pub pet: Pet,
    store: Store,
    pub screen: Screen,
    pub message: Option<(String, f32)>,
    /// Seconds since the app started.
    pub t: f32,
    /// Unix seconds, refreshed every tick.
    pub now: u64,
    pub x_off: f32,
    pub reaction: Option<(Reaction, f32)>,
    pub particles: Vec<Particle>,
    blink_until: f32,
    next_blink: f32,
    target_x: f32,
    next_wander: f32,
    next_sim: f32,
    next_save: f32,
    next_z: f32,
    rng: ThreadRng,
    quit: bool,
}

impl App {
    pub fn new(pet: Option<Pet>, store: Store) -> App {
        let now = now_secs();
        let first = pet.is_none();
        let mut pet = pet.unwrap_or_else(|| Pet::new("", now));
        pet.simulate(now);
        let mut app = App {
            pet,
            store,
            screen: Screen::Home,
            message: None,
            t: 0.0,
            now,
            x_off: 0.0,
            reaction: None,
            particles: Vec::new(),
            blink_until: 0.0,
            next_blink: 2.0,
            target_x: 0.0,
            next_wander: 3.0,
            next_sim: 1.0,
            next_save: 30.0,
            next_z: 0.0,
            rng: rand::rng(),
            quit: false,
        };
        if first {
            app.screen = Screen::Rename {
                input: String::new(),
                first: true,
            };
        } else {
            app.greet();
        }
        app
    }

    fn greet(&mut self) {
        let p = &self.pet;
        let msg = match p.mood() {
            _ if p.stage(self.now) == Stage::Egg => "The egg wiggles…".to_string(),
            Mood::Sleeping => format!("{} is fast asleep. zZ", p.name),
            Mood::Happy => format!("{} is so happy to see you!", p.name),
            Mood::Sick => format!("{} isn't feeling well…", p.name),
            Mood::Hungry => format!("{} missed you — and is starving!", p.name),
            Mood::Sad => format!("{} was lonely without you.", p.name),
            _ => format!("{} waves hello!", p.name),
        };
        self.say(msg);
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let mut last = Instant::now();
        while !self.quit {
            terminal.draw(|f| ui::draw(f, &self))?;
            if event::poll(Duration::from_millis(33))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.on_key(key);
            }
            let now = Instant::now();
            let dt = (now - last).as_secs_f32().min(0.25);
            last = now;
            self.tick(dt);
        }
        self.save();
        Ok(())
    }

    fn save(&mut self) {
        if self.pet.name.is_empty() {
            return;
        }
        if let Err(e) = self.store.save(&self.pet) {
            self.say(format!("Couldn't save: {e}"));
        }
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.message = Some((msg.into(), self.t + 4.0));
    }

    pub fn stage(&self) -> Stage {
        self.pet.stage(self.now)
    }

    /// The face to draw right now.
    pub fn expr(&self) -> Expr {
        if let Some((r, _)) = self.reaction {
            return match r {
                Reaction::Eat => Expr::Eating((self.t * 6.0) as i32 % 2 == 0),
                Reaction::Love => Expr::Love,
                Reaction::Sparkle => Expr::Happy,
                Reaction::Nope => Expr::Pout,
                Reaction::Cheer => Expr::Cheer,
            };
        }
        let mood = self.pet.mood();
        if mood != Mood::Sleeping && self.t < self.blink_until {
            return Expr::Blink;
        }
        mood.into()
    }

    /// How many rows the pet is hopping up.
    pub fn bounce(&self) -> i32 {
        let rate = match (self.reaction, self.pet.mood()) {
            (Some((Reaction::Cheer | Reaction::Love, _)), _) => 4.0,
            (Some(_), _) => return 0,
            (None, Mood::Happy) => 2.0,
            (None, Mood::Content) => 1.0,
            _ => return 0,
        };
        ((self.t * rate) as i32 % 2 == 1) as i32
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        let screen = std::mem::replace(&mut self.screen, Screen::Home);
        self.screen = self.handle(screen, key.code);
    }

    fn handle(&mut self, screen: Screen, code: KeyCode) -> Screen {
        match screen {
            Screen::Home => self.home_key(code),
            Screen::Feed(i) => match menu_key(code, i, Food::ALL.len()) {
                Menu::Move(i) => Screen::Feed(i),
                Menu::Back => Screen::Home,
                Menu::Pick(i) => {
                    let r = self.pet.feed(Food::ALL[i], self.now);
                    self.act(r, Reaction::Eat);
                    Screen::Home
                }
            },
            Screen::Play(i) => match menu_key(code, i, GameKind::ALL.len()) {
                Menu::Move(i) => Screen::Play(i),
                Menu::Back => Screen::Home,
                Menu::Pick(i) => {
                    let kind = GameKind::ALL[i];
                    let face = art::mini(Expr::Happy);
                    Screen::Game {
                        kind,
                        game: kind.start(face, self.rng.random()),
                        ended: None,
                    }
                }
            },
            Screen::Game {
                kind,
                mut game,
                ended,
            } => {
                if code == KeyCode::Esc {
                    self.say("Game over — no coins this time.");
                    return Screen::Home;
                }
                game.key(code);
                Screen::Game { kind, game, ended }
            }
            Screen::Results { .. }
                if matches!(code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) =>
            {
                Screen::Home
            }
            s @ Screen::Results { .. } => s,
            Screen::Rename { mut input, first } => match code {
                KeyCode::Enter | KeyCode::Esc => {
                    let name = input.trim();
                    if !name.is_empty() && code == KeyCode::Enter {
                        self.pet.name = name.to_string();
                    } else if first || self.pet.name.is_empty() {
                        self.pet.name = "Mochi".to_string();
                    }
                    if first {
                        self.say(format!("{} the egg is warm and wiggly…", self.pet.name));
                    } else {
                        self.say(format!("Hello, {}!", self.pet.name));
                    }
                    self.save();
                    Screen::Home
                }
                KeyCode::Backspace => {
                    input.pop();
                    Screen::Rename { input, first }
                }
                KeyCode::Char(c) if !c.is_control() && input.chars().count() < NAME_MAX => {
                    input.push(c);
                    Screen::Rename { input, first }
                }
                _ => Screen::Rename { input, first },
            },
            Screen::Help => Screen::Home,
        }
    }

    fn home_key(&mut self, code: KeyCode) -> Screen {
        let now = self.now;
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('f') => return Screen::Feed(0),
            KeyCode::Char('p') => match self.pet.can_play(now) {
                Ok(()) => return Screen::Play(0),
                Err(e) => self.act(Err(e), Reaction::Nope),
            },
            KeyCode::Char('c') => {
                let r = self.pet.clean(now);
                self.act(r, Reaction::Sparkle);
            }
            KeyCode::Char('s') => {
                let r = self.pet.toggle_sleep(now);
                self.act(r, Reaction::Sparkle);
                if self.pet.asleep {
                    self.reaction = None;
                    self.particles.clear();
                }
            }
            KeyCode::Char('m') => {
                let r = self.pet.medicine(now);
                self.act(r, Reaction::Nope);
            }
            KeyCode::Char('h') | KeyCode::Char(' ') => {
                let r = self.pet.hug(now);
                self.act(r, Reaction::Love);
            }
            KeyCode::Char('n') => {
                return Screen::Rename {
                    input: self.pet.name.clone(),
                    first: false,
                };
            }
            KeyCode::Char('?') => return Screen::Help,
            _ => {}
        }
        Screen::Home
    }

    fn act(&mut self, r: Result<String, String>, reaction: Reaction) {
        match r {
            Ok(msg) => {
                self.react(reaction);
                self.say(msg);
                self.save();
            }
            Err(msg) => {
                if self.stage() != Stage::Egg && !self.pet.asleep {
                    self.react(Reaction::Nope);
                }
                self.say(msg);
            }
        }
    }

    fn react(&mut self, r: Reaction) {
        self.reaction = Some((r, self.t + 1.6));
        let (chars, color, n): (&[char], Color, usize) = match r {
            Reaction::Love => (&['♥'], Color::Rgb(255, 105, 140), 4),
            Reaction::Sparkle => (&['*', '+', '·'], Color::Rgb(255, 230, 140), 7),
            Reaction::Eat => (&['.', ','], Color::Rgb(230, 200, 150), 5),
            Reaction::Cheer => (&['*', '+', '♥', '♪'], Color::Rgb(180, 220, 255), 8),
            Reaction::Nope => return,
        };
        for _ in 0..n {
            let ch = chars[self.rng.random_range(0..chars.len())];
            self.spawn(ch, color);
        }
    }

    fn spawn(&mut self, ch: char, color: Color) {
        self.particles.push(Particle {
            x: self.rng.random_range(-6.0..6.0),
            y: self.rng.random_range(0.0..3.0),
            vx: self.rng.random_range(-1.5..1.5),
            vy: self.rng.random_range(-3.5..-1.5),
            ch,
            color,
            life: self.rng.random_range(0.8..1.6),
        });
    }

    fn tick(&mut self, dt: f32) {
        self.t += dt;
        self.now = now_secs();

        if self.t >= self.next_sim {
            self.next_sim = self.t + 1.0;
            self.live();
        }
        if self.t >= self.next_save {
            self.next_save = self.t + 30.0;
            self.save();
        }
        if self.message.as_ref().is_some_and(|m| self.t > m.1) {
            self.message = None;
        }
        if self.reaction.is_some_and(|r| self.t > r.1) {
            self.reaction = None;
        }

        if self.t >= self.next_blink {
            self.blink_until = self.t + 0.15;
            self.next_blink = self.t + self.rng.random_range(2.0..6.0);
        }
        let wanders = !self.pet.asleep && !self.pet.sick() && self.stage() != Stage::Egg;
        if self.t >= self.next_wander {
            self.next_wander = self.t + self.rng.random_range(2.0..5.0);
            self.target_x = if wanders && self.rng.random_bool(0.6) {
                self.rng.random_range(-10.0..10.0)
            } else {
                0.0
            };
        }
        if self.reaction.is_none() || !wanders {
            let step = 5.0 * dt;
            self.x_off += (self.target_x - self.x_off).clamp(-step, step);
        }
        if self.reaction.is_some_and(|r| r.0 == Reaction::Nope) {
            // Shake the head.
            self.x_off = ((self.t * 20.0).sin()).round();
        }

        if self.pet.asleep && self.t >= self.next_z {
            self.next_z = self.t + 1.2;
            self.particles.push(Particle {
                x: 4.0,
                y: 0.0,
                vx: 1.0,
                vy: -1.2,
                ch: if self.rng.random_bool(0.5) { 'z' } else { 'Z' },
                color: Color::Rgb(150, 160, 220),
                life: 3.0,
            });
        }
        for p in &mut self.particles {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.life -= dt;
        }
        self.particles.retain(|p| p.life > 0.0);

        let finished = match &mut self.screen {
            Screen::Game { kind, game, ended } => {
                game.tick(dt);
                if game.over() && ended.is_none() {
                    *ended = Some(self.t + 1.2);
                }
                ended
                    .is_some_and(|e| self.t >= e)
                    .then(|| (*kind, game.score()))
            }
            _ => None,
        };
        if let Some((kind, score)) = finished {
            let coins = kind.coins(score);
            let record = self.pet.finish_game(kind.id(), score, coins);
            self.save();
            self.react(Reaction::Cheer);
            self.screen = Screen::Results {
                kind,
                score,
                coins,
                record,
            };
        }
    }

    /// Advances the pet's life and narrates anything that happened.
    fn live(&mut self) {
        let before = self.stage();
        let c = self.pet.simulate(self.now);
        let after = self.stage();
        let name = self.pet.name.clone();
        if before != after {
            self.react(Reaction::Cheer);
            self.say(if before == Stage::Egg {
                format!("Crack! {name} hatched! Say hi!")
            } else {
                format!("{name} grew into a {}!", after.name())
            });
            self.save();
        } else if c.got_sick {
            self.say(format!("{name} is feeling sick… try some medicine (m)."));
        } else if c.new_poops > 0 {
            self.say(format!("Oops — {name} made a little mess. (c to clean)"));
        } else if c.fell_asleep {
            self.say(format!("{name} got so tired they dozed off."));
        } else if c.woke_up {
            self.say(format!("{name} woke up well rested!"));
        }
    }
}

enum Menu {
    Move(usize),
    Pick(usize),
    Back,
}

fn menu_key(code: KeyCode, i: usize, len: usize) -> Menu {
    match code {
        KeyCode::Up | KeyCode::Char('k') => Menu::Move((i + len - 1) % len),
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => Menu::Move((i + 1) % len),
        KeyCode::Enter | KeyCode::Char(' ') => Menu::Pick(i),
        KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) < len => {
            Menu::Pick(c as usize - '1' as usize)
        }
        KeyCode::Esc | KeyCode::Char('q') => Menu::Back,
        _ => Menu::Move(i),
    }
}
