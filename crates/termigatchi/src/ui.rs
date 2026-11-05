// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Drawing every screen.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::app::{App, NAME_MAX, Screen};
use crate::art;
use crate::canvas::Canvas;
use crate::games::GameKind;
use crate::pet::{EGG_SECS, Food, Stage, fmt_age};

const ACCENT: Color = Color::Rgb(255, 135, 175);
const DIM: Color = Color::Rgb(120, 120, 140);
const TEXT: Color = Color::Rgb(220, 220, 230);
const GOLD: Color = Color::Rgb(255, 215, 110);
const MIN_W: u16 = 64;
const MIN_H: u16 = 22;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < MIN_W || area.height < MIN_H {
        let msg = format!(
            "termigatchi needs a {MIN_W}×{MIN_H} terminal\n(this one is {}×{})",
            area.width, area.height
        );
        f.render_widget(
            Paragraph::new(msg).alignment(Alignment::Center).fg(ACCENT),
            area,
        );
        return;
    }
    if let Screen::Game { kind, game, .. } = &app.screen {
        draw_game(f, area, app, *kind, game.as_ref());
        return;
    }
    draw_home(f, area, app);
    match &app.screen {
        Screen::Feed(i) => draw_feed(f, area, app, *i),
        Screen::Play(i) => draw_play(f, area, app, *i),
        Screen::Results {
            kind,
            score,
            coins,
            record,
        } => draw_results(f, area, app, *kind, *score, *coins, *record),
        Screen::Rename { input, first } => draw_rename(f, area, input, *first),
        Screen::Help => draw_help(f, area),
        Screen::Home | Screen::Game { .. } => {}
    }
}

fn pet_color(app: &App) -> Color {
    if app.pet.sick() {
        return Color::Rgb(170, 200, 120);
    }
    match app.stage() {
        Stage::Egg => Color::Rgb(245, 240, 220),
        Stage::Baby => Color::Rgb(255, 214, 165),
        Stage::Kid => Color::Rgb(152, 240, 200),
        Stage::Teen => Color::Rgb(135, 206, 250),
        Stage::Adult => Color::Rgb(203, 166, 247),
    }
}

fn draw_home(f: &mut Frame, area: Rect, app: &App) {
    let [title, main, message, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [room, side] =
        Layout::horizontal([Constraint::Min(30), Constraint::Length(30)]).areas(main);

    let p = &app.pet;
    let name = if p.name.is_empty() { "???" } else { &p.name };
    let header = Line::from(vec![
        " termigatchi ".bold().fg(Color::Black).bg(ACCENT),
        "  ".into(),
        name.to_string().bold().fg(TEXT),
        format!(
            " · {} · {} old",
            app.stage().name(),
            fmt_age(p.age(app.now))
        )
        .fg(DIM),
    ]);
    let right = Line::from(vec![
        format!("Lv {} ", p.level()).fg(TEXT),
        format!(" ¢{} ", p.coins).fg(GOLD).bold(),
    ]);
    f.render_widget(Paragraph::new(header), title);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), title);

    draw_room(f, room, app);
    draw_stats(f, side, app);

    if let Some((msg, _)) = &app.message {
        f.render_widget(Paragraph::new(format!(" {msg}")).fg(TEXT), message);
    }
    let keys: &[(&str, &str)] = &[
        ("f", "feed"),
        ("p", "play"),
        ("c", "clean"),
        ("s", if p.asleep { "wake" } else { "sleep" }),
        ("m", "medicine"),
        ("h", "hug"),
        ("n", "rename"),
        ("?", "help"),
        ("q", "quit"),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (k, label) in keys {
        spans.push(k.to_string().bold().fg(ACCENT));
        spans.push(format!(" {label}  ").fg(DIM));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), footer);
}

fn draw_room(f: &mut Frame, area: Rect, app: &App) {
    let asleep = app.pet.asleep;
    let title = if asleep { " lights out " } else { " home " };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if asleep { Color::Rgb(70, 70, 100) } else { DIM }))
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut c = Canvas::new(inner.width, inner.height);
    // A window: sun and drifting cloud by day, moon and stars at night.
    let frame = Style::default().fg(Color::Rgb(150, 150, 170));
    c.sprite(
        3,
        1,
        &[".--------.", "|        |", "|        |", "'--------'"],
        frame,
    );
    if asleep {
        let star = Style::default().fg(Color::Rgb(230, 230, 160));
        c.put(5, 2, '(', star);
        let twinkle = if (app.t * 1.5) as i32 % 2 == 0 {
            '*'
        } else {
            '+'
        };
        c.put(9, 2, twinkle, star);
        c.put(7, 3, '.', star);
        c.put(10, 3, '.', star);
    } else {
        c.put(10, 2, 'o', Style::default().fg(GOLD));
        let drift = (app.t * 0.3) as i32 % 8;
        c.text(4 + drift.min(6), 3, "~~", Style::default().fg(TEXT));
        c.put(11, 3, '|', frame);
    }
    // A little potted plant in the corner.
    let leaf = Style::default().fg(Color::Rgb(120, 200, 120));
    c.sprite(2, c.h - 3, &[" \\|/", "  |"], leaf);
    c.text(
        2,
        c.h - 1,
        " [_]",
        Style::default().fg(Color::Rgb(190, 120, 90)),
    );

    // Poops, lined up from the right.
    let brown = Style::default().fg(Color::Rgb(170, 120, 80));
    for i in 0..app.pet.poops as i32 {
        let x = c.w - 6 - i * 6;
        c.sprite(x, c.h - 2, &[" ( ", "(__)"], brown);
        let stink = if (app.t * 2.0) as i32 % 2 == 0 {
            "~ "
        } else {
            " ~"
        };
        c.text(
            x + 1,
            c.h - 3,
            stink,
            Style::default().fg(Color::Rgb(140, 170, 100)),
        );
    }

    let stage = app.stage();
    let age = app.pet.age(app.now);
    let sprite = art::sprite(
        stage,
        app.expr(),
        stage == Stage::Egg && age >= EGG_SECS * 2 / 3,
    );
    let sw = sprite[0].chars().count() as i32;
    let sh = sprite.len() as i32;
    let mut x = (c.w - sw) / 2 + app.x_off.round() as i32;
    if stage == Stage::Egg && age >= EGG_SECS / 3 {
        x += [0, -1, 0, 1][(app.t * 6.0) as usize % 4];
    }
    let y = c.h - sh - app.bounce();
    c.sprite(x, y, &sprite, Style::default().fg(pet_color(app)));

    let (cx, top) = (x + sw / 2, y);
    for p in &app.particles {
        c.put(
            cx + p.x.round() as i32,
            top + p.y.round() as i32,
            p.ch,
            Style::default().fg(p.color),
        );
    }
    if stage == Stage::Egg {
        let left = EGG_SECS.saturating_sub(age);
        c.text_centered(
            1,
            &format!("hatching in {left}s…"),
            Style::default().fg(DIM),
        );
    }
    if asleep {
        c.tint(Style::default().add_modifier(Modifier::DIM));
    }
    f.render_widget(Paragraph::new(c.lines()), inner);
}

fn bar(label: &str, value: f32) -> Line<'static> {
    let filled = (value / 10.0).round() as usize;
    let color = if value >= 60.0 {
        Color::Rgb(140, 230, 160)
    } else if value >= 30.0 {
        GOLD
    } else {
        Color::Rgb(255, 110, 110)
    };
    Line::from(vec![
        format!(" {label:<7}").fg(TEXT),
        "█".repeat(filled).fg(color),
        "░".repeat(10 - filled).fg(Color::Rgb(60, 60, 75)),
        format!(" {:>3}", value.round() as u32).fg(DIM),
    ])
}

fn draw_stats(f: &mut Frame, area: Rect, app: &App) {
    let p = &app.pet;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(" needs ");
    let mut lines = vec![
        Line::raw(""),
        bar("Food", p.fullness),
        bar("Fun", p.happiness),
        bar("Energy", p.energy),
        bar("Clean", p.hygiene),
        bar("Health", p.health),
        Line::raw(""),
    ];
    if app.stage() == Stage::Egg {
        lines.push(Line::from(vec![
            " mood    ".fg(TEXT),
            "cozy in its shell".fg(ACCENT),
        ]));
    } else {
        let mood = p.mood();
        lines.push(Line::from(vec![
            " mood    ".fg(TEXT),
            mood.name().fg(ACCENT).bold(),
        ]));
        let needs = p.needs();
        if !needs.is_empty() {
            lines.push(Line::from(vec![
                " wants   ".fg(TEXT),
                needs.join(", ").fg(GOLD),
            ]));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        " meals   ".fg(TEXT),
        p.meals.to_string().fg(DIM),
    ]));
    lines.push(Line::from(vec![
        " games   ".fg(TEXT),
        p.games_played.to_string().fg(DIM),
    ]));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn popup(f: &mut Frame, area: Rect, w: u16, h: u16, title: &str) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(format!(" {title} "))
        .title_bottom(
            Line::from(" ↑↓ choose · enter pick · esc back ")
                .fg(DIM)
                .centered(),
        );
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    inner
}

fn menu_line(selected: bool, n: usize, title: &str, detail: String) -> Line<'static> {
    let marker = if selected { "▸ " } else { "  " };
    let style = if selected {
        Style::default().fg(ACCENT).bold()
    } else {
        Style::default().fg(TEXT)
    };
    Line::from(vec![
        Span::styled(format!(" {marker}{n} {title:<13}"), style),
        Span::styled(detail, Style::default().fg(DIM)),
    ])
}

fn draw_feed(f: &mut Frame, area: Rect, app: &App, sel: usize) {
    let inner = popup(f, area, 48, 9, "feed");
    let mut lines = vec![Line::raw("")];
    for (i, food) in Food::ALL.iter().enumerate() {
        lines.push(menu_line(i == sel, i + 1, food.name(), food.blurb()));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(format!("   you have ¢{}", app.pet.coins)).fg(GOLD));
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_play(f: &mut Frame, area: Rect, app: &App, sel: usize) {
    let inner = popup(f, area, 56, 11, "play");
    let mut lines = vec![Line::raw("")];
    for (i, g) in GameKind::ALL.iter().enumerate() {
        lines.push(menu_line(i == sel, i + 1, g.title(), g.blurb().to_string()));
    }
    lines.push(Line::raw(""));
    let g = GameKind::ALL[sel];
    lines.push(
        Line::from(format!(
            "   best: {}   ·   {}",
            app.pet.best(g.id()),
            g.controls()
        ))
        .fg(GOLD),
    );
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_game(f: &mut Frame, area: Rect, app: &App, kind: GameKind, game: &dyn crate::games::Game) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Line::from(format!(" {} ", kind.title())).bold())
        .title(
            Line::from(format!(" best {} ", app.pet.best(kind.id())))
                .fg(GOLD)
                .right_aligned(),
        )
        .title_bottom(
            Line::from(format!(" {} · esc quit ", kind.controls()))
                .fg(DIM)
                .centered(),
        );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut c = Canvas::new(inner.width, inner.height);
    game.draw(&mut c);
    f.render_widget(Paragraph::new(c.lines()), inner);
}

#[allow(clippy::too_many_arguments)]
fn draw_results(
    f: &mut Frame,
    area: Rect,
    app: &App,
    kind: GameKind,
    score: u32,
    coins: u32,
    record: bool,
) {
    let inner = popup(f, area, 40, 9, kind.title());
    let mut lines = vec![
        Line::raw(""),
        Line::from(format!("score {score}"))
            .bold()
            .fg(TEXT)
            .centered(),
    ];
    if record {
        lines.push(Line::from("★ new best! ★").fg(GOLD).bold().centered());
    } else {
        lines.push(
            Line::from(format!("best {}", app.pet.best(kind.id())))
                .fg(DIM)
                .centered(),
        );
    }
    lines.push(
        Line::from(format!("+¢{coins}  ·  {} had fun!", app.pet.name))
            .fg(ACCENT)
            .centered(),
    );
    lines.push(Line::raw(""));
    lines.push(Line::from("enter to go home").fg(DIM).centered());
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_rename(f: &mut Frame, area: Rect, input: &str, first: bool) {
    let (title, prompt) = if first {
        ("a new friend", "You found an egg! What will you name it?")
    } else {
        ("rename", "What should your pet be called?")
    };
    let inner = popup(f, area, 50, 8, title);
    let lines = vec![
        Line::raw(""),
        Line::from(prompt).fg(TEXT).centered(),
        Line::raw(""),
        Line::from(vec![
            "▸ ".fg(ACCENT),
            input.to_string().bold().fg(TEXT),
            "▏".fg(ACCENT).add_modifier(Modifier::SLOW_BLINK),
            " ".repeat(NAME_MAX.saturating_sub(input.chars().count()))
                .into(),
        ])
        .centered(),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let inner = popup(f, area, 62, 19, "help");
    let text = vec![
        Line::raw(""),
        Line::from("Your pet lives on even while termigatchi is closed:").fg(TEXT),
        Line::from("it gets hungry, tired, and messy in real time.").fg(TEXT),
        Line::raw(""),
        Line::from(vec![
            "f ".fg(ACCENT).bold(),
            "feed — rice balls are free, treats cost coins".fg(DIM),
        ]),
        Line::from(vec![
            "p ".fg(ACCENT).bold(),
            "play a minigame to earn coins and fun".fg(DIM),
        ]),
        Line::from(vec![
            "c ".fg(ACCENT).bold(),
            "clean up poop and give a bath".fg(DIM),
        ]),
        Line::from(vec!["s ".fg(ACCENT).bold(), "lights out / wake up".fg(DIM)]),
        Line::from(vec![
            "m ".fg(ACCENT).bold(),
            "medicine, when your pet is sick".fg(DIM),
        ]),
        Line::from(vec!["h ".fg(ACCENT).bold(), "hug (or space)".fg(DIM)]),
        Line::raw(""),
        Line::from("Grows: Egg → Baby → Kid (1d) → Teen (3d) → Adult (7d)").fg(TEXT),
        Line::raw(""),
        Line::from("Keep it close: add `termigatchi status` to your prompt,").fg(TEXT),
        Line::from("or try `termigatchi feed` from any shell.").fg(TEXT),
    ];
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::pet::Pet;
    use crate::save::Store;

    fn render(app: &App) -> String {
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app_with(pet: Pet) -> App {
        let store = Store::at(std::env::temp_dir().join("termigatchi-ui-test/pet.json"));
        App::new(Some(pet), store)
    }

    #[test]
    fn home_shows_pet_and_needs() {
        let mut pet = Pet::new("Bean", crate::pet::now_secs() - 2 * crate::pet::DAY);
        pet.updated = crate::pet::now_secs();
        let app = app_with(pet);
        let screen = render(&app);
        assert!(screen.contains("Bean"), "{screen}");
        assert!(screen.contains("Kid"), "{screen}");
        assert!(screen.contains("Food"), "{screen}");
        assert!(screen.contains("'-...-'"), "{screen}");
    }

    #[test]
    fn first_run_asks_for_a_name() {
        let store = Store::at(std::env::temp_dir().join("termigatchi-ui-test-new/pet.json"));
        let app = App::new(None, store);
        let screen = render(&app);
        assert!(screen.contains("What will you name it?"), "{screen}");
        assert!(screen.contains("hatching in"), "{screen}");
    }
}
