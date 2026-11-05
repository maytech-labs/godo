# termigatchi

A tiny pet that lives in your terminal. It gets hungry, sleepy, and messy in
real time — even while the app is closed — so check in on it.

```
        .-"""-.
       /       \
      |  ^   ^  |
      | .  w  . |
       \       /
        '-...-'
         ^   ^
```

## Install

```sh
brew install maytech-labs/tap/termigatchi
```

Or from source, in a clone of godo: `cargo install --locked --path crates/termigatchi`

### Trying it from source

From the root of godo:

```sh
cargo run --release -p termigatchi            # or: godo dev termigatchi
```

To try it without touching your real pet, point it at a throwaway save:

```sh
TERMIGATCHI_HOME=/tmp/pet cargo run --release -p termigatchi
```

Growing up takes days. To skip ahead, quit and subtract seconds from `born`
in the save (`termigatchi where` prints its path): 86400 for a Kid, 259200
for a Teen, 604800 for an Adult.

## Play

Run `termigatchi`. You'll find an egg; name it and it hatches in a few
seconds.

| key | does |
| --- | --- |
| `f` | feed — rice balls are free, cookies and cake cost coins |
| `p` | play a minigame to earn coins |
| `c` | clean up poop |
| `s` | lights out / wake up |
| `m` | medicine, when it's sick |
| `h` / space | hug |
| `n` | rename |
| `?` | help |

It grows from Egg → Baby → Kid (1 day) → Teen (3 days) → Adult (7 days).
Neglect makes it sick and sad, but it never dies.

### Minigames

- **Which Way?** — guess where your pet will look.
- **Snack Rain** — catch falling snacks, dodge the rocks.
- **Hop!** — jump the cacti; it keeps getting faster.
- **Echo** — repeat the growing pattern.
- **High or Low** — is the next card higher or lower?

## From your shell

```sh
termigatchi status        # (^w^) Mochi ♥♥♥♥♡ hungry
termigatchi feed cookie
termigatchi clean
termigatchi hug
```

Keep it in your prompt (zsh):

```sh
setopt PROMPT_SUBST
RPROMPT='$(termigatchi status)'
```

`status` prints nothing until you have a pet, so it's safe to add early.

## Save file

`~/Library/Application Support/termigatchi/pet.json` on macOS,
`$XDG_DATA_HOME/termigatchi/pet.json` (or `~/.local/share/…`) elsewhere.
`termigatchi where` prints it; set `TERMIGATCHI_HOME` to keep it somewhere
else.

## Releasing

See [CONTRIBUTING.md](../../CONTRIBUTING.md#releasing-termigatchi). The first
time, copy `packaging/termigatchi.rb` into `Formula/` in
[maytech-labs/homebrew-tap](https://github.com/maytech-labs/homebrew-tap);
later releases update it automatically.
