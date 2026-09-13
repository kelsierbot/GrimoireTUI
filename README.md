# GrimoireTUI

A terminal writing desk for novels. Rust + [Ratatui](https://ratatui.rs).

**[grimoire.joshking.ai](https://grimoire.joshking.ai)**

```
┌ MANUSCRIPT ────────────┐┌ Chapter One / Gravel ─────────────────────────┐
│▾ Part One        2,120 ││                                               │
│  ▾ Chapter One   2,120 ││  The caretaker was waiting by her car when    │
│    • The Archive 1,240 ││  she came back out, which meant he had been   │
│    ● Gravel        880 ││  watching the door the whole time she was     │
│  ▸ Chapter Two       0 ││  inside.                                      │
│── NOTES ───────────────││                                               │
│▾ Characters         64 ││  "You'll want to come back in daylight,"      │
│  • Wren             64 ││  he said.                                     │
│                        ││                                               │
│                        ││  It was daylight.                             │
└────────────────────────┘└───────────────────────────────────────────────┘
 2,120 / 80,000 ▓░░░░░░░░░  today 412           Tab pane  ^S save  ^Q quit
```

## Why it exists

Every open-source novel tool makes one of two mistakes: it traps your manuscript
in a database, or it's a text editor with no idea what a manuscript is.
GrimoireTUI does neither.

**Your novel is a folder of plain Markdown.** No index file, no database, no
lock-in — so there's no central thing to conflict when you sync across machines.
If this project disappears tomorrow you still have your book: readable by `cat`,
editable in Obsidian, diffable in git, line by line.

## Install

Requires Rust 1.85+ (edition 2024).

```sh
git clone https://github.com/kelsierbot/GrimoireTUI
cd GrimoireTUI
cargo install --path .
grimoire
```

`cargo install` puts the binary in `~/.cargo/bin`, so make sure that is on your
`PATH`.

Typing `grimoire` on its own opens the current directory if it is a manuscript,
otherwise the last one you had open, otherwise it starts a fresh one in
`~/Documents/Grimoire`. To be explicit:

```sh
grimoire ~/novels/the-archive     open a specific manuscript
grimoire new ~/novels/next-one    start one
```

The crate is `grimoire-tui`; the binary it installs is `grimoire`.
A crates.io release is coming.

## Project layout

```
novel.toml                                       title, author, word targets
manuscript/01-part-one/01-chapter-one/01-the-archive.md
notes/characters/wren.md
.grimoire/progress.toml                          session tracking (gitignored)
```

Directories are containers — parts, chapters, any depth you like. `.md` files
are scenes. Order comes from the filename; a leading `01-` is stripped for
display. Scene metadata lives in YAML frontmatter:

```yaml
---
title: Gravel
pov: Wren
status: draft
synopsis: Confronts the caretaker in the lot.
target: 1200
---
```

Frontmatter is **round-tripped verbatim**. GrimoireTUI reads a few keys and
never rewrites the block, so Obsidian and anything else can own fields it has
never heard of.

## Keys

| Key | |
|---|---|
| `Tab` | switch pane |
| `↑ ↓` / `j k` | move in tree |
| `→` / `l` / `Enter` | expand container, or open scene |
| `←` / `h` | collapse, or jump to parent |
| `Ctrl-S` | save all changed scenes |
| `Ctrl-Q` | quit (twice if unsaved) |
| `Esc` | leave editor, back to tree |
| `F2` / `F3` | start or pause the timer / reset it |
| `F4` `F5` `F6` | previous · play-pause · next |

Function keys work from either pane, so they can never eat a keystroke while
you're writing.

**On the modifier:** Grimoire accepts either `Ctrl` or `Cmd`, and the status bar
labels whichever your terminal can actually deliver. Cmd only reaches a terminal
application through the Kitty keyboard protocol — Ghostty, Kitty, WezTerm and
foot support it; Apple Terminal does not, and there is nothing an application
can do about that. Ctrl always works.

In the editor: type. Arrows, Home/End, PageUp/PageDown, Backspace, Delete.
Vertical movement is **visual** — Down moves one screen row inside a wrapped
paragraph rather than jumping a whole paragraph, which is the only behaviour
that feels right for prose.

## The clearing

Below the manuscript tree is a forest, and the forest is the timer.

```
┌ focus · 18:04 ─────────────┐
│  ·                         │
│         ☀        ·         │
│    ▲      ▲       ▲        │
│   ▲▲▲    ▲▲▲     ▲▲▲    ▲  │
│  ▲▲▲▲▲  ▲▲▲▲▲   ▲▲▲▲▲  ▲▲▲ │
│    ┃      ┃       ┃     ┃  │
│   (\_/)            (\_/)   │
│   (•ᴥ•)    ❀  ✿    (•ᴥ•)   │
│▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔│
└────────────────────────────┘
```

The sun's **position is the clock** — it crosses the sky over a 25 minute focus
session, so you read the time remaining off the light instead of watching a
countdown. Break time is night: the moon rises, the flowers close, and fireflies
come out between the rabbits. The rabbits flick their ears on mutually-prime
cycles so they never move in lockstep.

`F2` starts and pauses. `F3` resets.

## Music

There is no official YouTube Music API, and anything that extracts stream URLs
both violates the terms and side-steps the subscription you already pay for. So
Grimoire never plays audio.

Instead it drives [YTMDesktop](https://github.com/ytmdesktop/ytmdesktop)'s
Companion Server, which is already signed into your real account. Your
playlists, your Premium, your playback — Grimoire just presses the buttons.

```sh
brew install --cask ytmdesktop-youtube-music
# enable Settings → Integrations → Companion Server, then:
grimoire music-auth
```

Pairing prints a code you approve inside YTMDesktop; the token lands in
`~/.config/grimoire/music.toml`. With no token configured the pane stays inert
and never touches the network.

## Status

v0.1. It opens a manuscript, shows you the shape of it, tracks your words, and
lets you write.

Not yet: command palette, corkboard view, git sync, scene reordering,
create/delete from inside the app, search, spellcheck. `pov` and `status` are
already parsed out of frontmatter and waiting on the corkboard.

## Repo layout

```
src/          the app
example/      a tiny sample manuscript to try it against
site/         the landing page (Astro) behind grimoire.joshking.ai
```

## License

MIT
