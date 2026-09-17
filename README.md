<img src="assets/banner.svg" alt="grimoire — a terminal writing desk for novels" width="820">

**[grimoire.joshking.ai](https://grimoire.joshking.ai)** · Rust + [Ratatui](https://ratatui.rs) · MIT · macOS, Linux and Windows

A writing desk for novels that lives in your terminal. It knows what a
manuscript is — parts, chapters, scenes, word targets — and it keeps every word
of it as plain files you could read with `cat`.

---

## Why it exists

Every open-source novel tool makes one of two mistakes. It traps your
manuscript in a database, or it's a text editor with no idea what a manuscript
is.

Grimoire does neither. **The directory tree is the outline.** There is no index
file, so there is nothing central to conflict when you sync across machines.
Reordering is renaming. Backing up is copying a folder.

If this project is abandoned tomorrow you still have your book. That is the
whole design constraint, and everything else follows from it.

## Install

**Windows** — open PowerShell or Command Prompt and paste:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/kelsierbot/GrimoireTUI/releases/latest/download/grimoire-tui-installer.ps1 | iex"
```

**macOS and Linux** — in a terminal:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/kelsierbot/GrimoireTUI/releases/latest/download/grimoire-tui-installer.sh | sh
```

Then open a **new** terminal (on Windows, Windows Terminal is best) and run
`grimoire`. Nothing else to install — no Rust, no build tools. The installer
puts `grimoire` in `~/.cargo/bin` and adds that to your `PATH`. Builds are for
Windows x64, macOS (Apple Silicon and Intel) and Linux (x86_64 and ARM64);
every download is on the [releases page](https://github.com/kelsierbot/GrimoireTUI/releases).

**With Homebrew** (macOS or Linux):

```sh
brew install kelsierbot/tap/grimoire
```

`brew upgrade` then brings each new release.

Settings live in `~/.config/grimoire` (`%USERPROFILE%\.config\grimoire` on
Windows) and a first manuscript goes in `Documents/Grimoire`.

**Linux needs two things every desktop distro already has:** glibc 2.35 or
newer (Ubuntu 22.04, Debian 12, Fedora 36 and later), and ALSA's audio library.
Minimal installs, servers, containers and WSL can be missing the library, and
then `grimoire` stops with `libasound.so.2: cannot open shared object file`.
Install it and run `grimoire` again:

```sh
sudo apt install libasound2t64   # Ubuntu 24.04+, Debian 13+ (older: libasound2)
sudo dnf install alsa-lib        # Fedora
sudo pacman -S alsa-lib          # Arch
```

### From source

Requires Rust 1.85 or newer.

```sh
git clone https://github.com/kelsierbot/GrimoireTUI
cd GrimoireTUI
cargo install --path .
```

The crate is `grimoire-tui`; the binary it installs is `grimoire`. On Windows,
install Rust with [rustup](https://rustup.rs) and let it add the Visual Studio
C++ build tools.

```sh
grimoire                          open your manuscript
grimoire ~/novels/the-archive     open a specific one
grimoire new ~/novels/next-one    start one
grimoire music-setup              install + connect YouTube Music
```

With no arguments it opens the current directory if it's a manuscript,
otherwise the last one you had open, otherwise it creates one in
`~/Documents/Grimoire`.

## On disk

Folders are containers, at any depth. Files are scenes. Order comes from the
name — a leading `01-` sorts the file and is stripped for display.

```
the-archive/
├─ novel.toml                    title, author, word targets, part_label
├─ manuscript/
│  ├─ 01-Act-One/
│  │  ├─ 01-Chapter-One/
│  │  │  ├─ 01-Scene-One.md      a scene
│  │  │  ├─ 02-Scene-Two.md
│  │  │  └─ 03-Scene-Three.md
│  │  └─ 02-Chapter-Two/ …
│  ├─ 02-Act-Two/ …
│  └─ 03-Act-Three/ …
├─ notes/
│  ├─ 01-Characters/   02-Races/     03-Regions/   04-Magic-System/
│  └─ 05-Politics/     06-Religion/  07-Notes/     08-Research/
└─ .grimoire/                    session state, gitignored
   └─ trash/                     what you delete, still shown in the tree
```

**A new book arrives with its shape already standing:** three acts of nine
chapters, three scenes in each — twenty-seven chapters, eighty-one scenes, all
of them empty. The structure is a suggestion you rearrange; the writing is
never presumed. `part_label` in `novel.toml` is what this book calls its
largest division — `Act` to begin with, `Part` or `Book` if you prefer — and
the app says your word back everywhere it names one.

The notebook sections are numbered so they read in the order a world gets
built rather than alphabetically. A big book opens folded down to the first
scene, so launching shows you the shape rather than a hundred rows.

Scene metadata lives in YAML frontmatter:

```yaml
---
title: Gravel
pov: Wren
status: draft
synopsis: Confronts the caretaker in the lot.
target: 1200
---
```

Frontmatter is **round-tripped verbatim**. Grimoire reads a few keys and never
rewrites the block, so Obsidian and anything else can own fields it has never
heard of.

## Keys

| Key | |
|---|---|
| `Ctrl-K` | **find anything** — every action, scene, note and theme by name, with its key |
| `Tab` / `Shift-Tab` | cycle panes — tree, editor, the open note, clearing, music |
| `↑ ↓` · `j k` | move in the tree |
| `Enter` · `Space` | fold or unfold a container · open a scene |
| `→` `l` / `←` `h` | expand / collapse, or jump to parent |
| `n` | new scene — at the end of the selected chapter (a selected part's last chapter) |
| `c` | new chapter — named for you ("Chapter Two"), at the end of the part you're in |
| `p` | new part — named for you ("Part Two", or "Act Two" if `part_label` says so), at the end of the manuscript |
| `N` | new folder beside the selected one — for notes; in the manuscript it's a chapter or part |
| `r` | rename what's selected — the file is renamed to match and keeps its place |
| `d` | delete what's selected — asks first, then moves it to `.grimoire/trash` |
| `Alt-↑` `Alt-↓` · `K` `J` | move the scene or folder up or down — across chapters and acts at the ends; dragging a row in the tree does the same |
| `H` | the scene's history — every kept version, what changed, restore any |
| `b` | the corkboard |
| `/` | find in the whole book |
| `Ctrl-F` | find in the scene (`Ctrl-F` again: the whole book) · `Tab` to replace · `Ctrl-R` replace all |
| `Ctrl-Z` `Ctrl-Y` | undo · redo (`Ctrl-Shift-Z` too, where the terminal can send it) |
| `Ctrl-X` `Ctrl-C` | cut · copy the selection |
| `Ctrl-O` | open the note for the name under the cursor beside the scene |
| `F8` | spelling suggestions for this word, or the next misspelling |
| `Ctrl-S` | save now — autosave already does, two seconds after you stop typing |
| `Ctrl-Q` · `q` in the tree | quit, saving everything first |
| `Esc` | leave the editor |
| `F2` `F3` | start or pause the timer · reset |
| `F9` · `t` | themes |
| `←` `→` | in the clearing pane: switch view |
| `F4` `F5` `F6` | previous · play-pause · next (with music on) |
| `F7` | the music player — queue, your playlists, search, and every control (also `F1`, or `Enter` on the music pane) |

## Nothing is lost

- **Autosave.** A scene is written two seconds after you stop typing, and
  never sits unsaved longer than twenty. Every write goes through a temporary
  file, so a crash mid-save can't tear a scene in half. Closing the terminal
  window, logging out or `kill` saves first; so does quitting.
- **Recovery.** If a save fails (a full disk, a file another program has
  locked) or Grimoire crashes, the words go to `.grimoire/recovery/` and the
  next launch offers them back beside the saved version.
- **Undo.** A word at a time, per scene, kept when you switch scenes. Typing
  over a selection replaces it; a paste is one step.
- **History.** Every scene keeps dated copies as you work — at most every five
  minutes, plus how it was before today's first change and before anything
  drastic — in `.grimoire/history/`, as plain Markdown. `H` lists them with a
  word diff against now and restores any version (itself undoable).

## Finding and shaping

- **Find and replace** in a scene or across the book, grouped by act and
  chapter, with a preview before anything is replaced across scenes (each
  scene's previous version goes to its history first).
- **Name drift** (palette: *Check names*): spellings one or two letters away
  from a name in your notebook — *Kaelan* where the Characters note says
  *Kaelen* — with where they are, fixed in one go.
- **Spellcheck** underlines misspellings from a bundled en_US dictionary and
  never flags your notebook's names. Words you add go in `dictionary.txt` at
  the book's root. On unless turned off (palette).
- **Moving** a scene or folder renames only the files whose number changes,
  and rewrites `[[links]]` in your notes that pointed at them.
- **The corkboard** (`b`) shows an act as index cards chapter by chapter:
  POV (each character in its own colour), status, synopsis, words against
  target. `p` filters to one POV; `s`, `e` and `v` edit the card, changing only
  that line of the scene's frontmatter.

## The notebook and the prose

Names from your notebook — note titles, their `aliases:`, the distinctive
words of a longer title — show in the accent colour as you write. `Ctrl-O` on
one opens that note beside the scene, with **Appears in**: every scene that
mentions it. Nothing extra is stored; it's all read from the files.

## Readers, and other computers

- **Export** (palette): a Word document in standard manuscript format, an
  EPUB for phones and e-readers, and Markdown — the whole book or chosen acts —
  into `exports/`. Also `grimoire export [--docx] [--epub] [--md] [--parts 1,3]`.
- **Resume.** `.grimoire/resume.md` remembers the scene and paragraph you were
  at, so opening the book on any computer the folder reaches lands you there.
- **Writing sessions** (palette): turn on history for the book and each
  session is kept as a snapshot labelled like a diary line — *Tuesday evening ·
  Act Two · 1,240 words* — with what changed that session. It's Git inside the
  book folder; connect a remote and sessions back themselves up in the
  background. Needs Git installed.

Vertical movement in the editor is **visual, not logical** — Down moves one
screen row inside a wrapped paragraph rather than jumping the whole paragraph,
which is the only behaviour that feels right for prose.

Grimoire accepts `Ctrl` or `Cmd`, and the status bar labels whichever your
terminal can actually deliver. Cmd only reaches a terminal application through
the Kitty keyboard protocol — Ghostty, Kitty, WezTerm and foot support it,
Apple Terminal cannot send it at all. Ctrl always works.

## Themes

`F9` anywhere, or `t` from the tree. Nine presets — Grimoire, Gruvbox Dark,
Nord, Dracula, Solarized Dark, Catppuccin Mocha, Tokyo Night, Everforest and
**Lost Forest** — and `j`/`k` **previews each one live** as you move, so you
pick by looking rather than by name. `Enter` keeps it, `Esc` puts back what
you had.

Lost Forest is the one written for this app rather than borrowed. It is
green-led on purpose: `accent` drives focused borders, pane titles, the
open-scene marker and the progress bar, so a cyan accent would make the whole
interface read cyan. Cyan is kept for the two places it stays rare — the
break-time moon and the flowers — with a warm rust for warnings, since an
all-green palette otherwise hides its own alerts.

The last entry is **Custom…**, which opens a swatch editor: twelve named roles
(accent, text, dim, border, selection, warning, then the six the forest uses),
each showing its colour block and hex. Type six hex digits and it applies the
moment the sixth lands. It starts from whichever theme you were previewing, so
you can pick the closest preset and adjust from there.

Saved to `~/.config/grimoire/theme.toml`. Presets store only their name, so
they follow any future refinements; Custom stores every swatch.

## Mouse

It's a TUI, but the mouse works. Click a chapter to fold it, a scene to open
it, anywhere in the prose to put the caret there. Click the clearing to start
the timer, or the music pane to play and pause. The wheel scrolls whichever
pane is under the pointer without stealing focus.

**Drag to select.** Click and drag in the prose to select across lines;
`Ctrl-C` copies it (via `pbcopy`, `wl-copy`, `xclip` or `xsel`, whichever the
machine has). Any keystroke collapses the selection — it deliberately does not
delete it, because there is no undo yet and a stray key must never eat text.

Mouse capture does suppress the terminal's *own* drag-to-select. Grimoire's is
the replacement inside the prose pane; hold `Shift` while dragging if you want
the terminal's version anywhere else.

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

`←`/`→` cycles the pane through three views: **the clearing**; a **spectrum**
analyser that listens to whatever your Mac is playing (macOS 14.6+, once your
terminal is allowed under *Privacy & Security › Screen & System Audio
Recording › System Audio Recording Only*); and **growth** — a garden where
every 50 words you write today adds a stem, a leaf or a flower.

The sun's **position is the clock**. It crosses the sky over a twenty-five
minute session, so you read the time remaining off the light instead of
watching a number count down. Break time is night: the moon rises, the flowers
close, and fireflies come out between the rabbits.

## Music

Music is **off until you turn it on** — from the menu (`F1` → *Turn music on*)
or the palette. Setting a source up turns it on too. Four sources, chosen from
the menu (`F1` → Music source) and remembered.

| Source | How it works | Setup |
|---|---|---|
| **YouTube Music** | drives [th-ch/youtube-music](https://github.com/th-ch/youtube-music)'s API Server | `grimoire music-setup youtube-music` |
| **Spotify** | drives the desktop app — AppleScript on macOS, MPRIS on Linux (not Windows yet) | nothing to do |
| **Jellyfin** | **Grimoire plays it** — your files, your server | `grimoire music-setup jellyfin` |
| **Plex** | **Grimoire plays it** — your files, your server | `grimoire music-setup plex` |

### The player

`F7` opens it from anywhere. The top shows what's playing with a real progress
bar; below it are three tabs, switched with `Tab`:

- **Queue** — everything queued, following the playing track. `↑↓` to browse,
  `Enter` to jump to any song.
- **Playlists** — your YouTube Music library. `Enter` replaces the queue with a
  playlist and starts it; `a` queues the whole thing after the current song.
  `/` searches every public playlist on YouTube Music; `Esc` brings yours back.
- **Search** — `/`, type, `Enter`. `Enter` on a result plays it now; `a` plays
  it next. Songs and videos only.

Everywhere in the player: `space` pause · `←` `→` seek 10 s · `[` `]` previous /
next · `s` shuffle · `r` repeat · `+` `-` volume · `l` like · `Esc` close.

How playlists work, since the API Server has no "play playlist" call: Grimoire
reads the songs from YouTube Music's public web listing — so the playlist has
to be public or unlisted, and a private one tells you so — then queues them in
the app one at a time, first song first so the music starts straight away.
Playlists past 300 songs are cut there. Queue browsing also works for Jellyfin
and Plex; Spotify gets the controls but doesn't share its queue.

The split matters. There is no official YouTube Music API, and anything that
extracts stream URLs both violates the terms and side-steps the subscription
you already pay for — so for YouTube Music and Spotify, Grimoire never touches
audio. It presses buttons on a player you already run.

Jellyfin and Plex are the opposite case: your own files on your own hardware,
no terms to violate and no subscription to bypass. So Grimoire plays those
itself and nothing else has to be running.

**Spotify needs no setup at all.** Its Web API would mean OAuth, a registered
application and a refresh-token dance for the privilege of pressing pause. Both
platforms already expose the running player locally, so there is nothing to
sign into and no token to store. On Linux it wants `playerctl`. Windows has
no equivalent Grimoire can use yet, so Spotify is macOS and Linux only.

**YouTube Music setup** installs the app for you on all three. On macOS and
Linux it asks whether the machine is x86 or ARM (Enter takes what it detects);
on Windows the installer picks the right build itself, so it doesn't ask.

**Jellyfin** signs in with the same username and password you use for the web
UI — no hunting through the dashboard for an API key. **Plex** needs an
`X-Plex-Token`, and the setup flow tells you where to find it.

Audio playback is a default cargo feature. On Linux it needs ALSA headers
(`libasound2-dev`); build with `--no-default-features` to drop it and keep the
remote-control sources.

## Status

**v0.2 (unreleased).** Everything above: autosave, recovery, undo and history;
the palette; find and replace; spellcheck and name drift; moving scenes; the
corkboard; the codex; export; resume and writing sessions.

Limits: spellcheck is English only, and Spotify isn't supported on Windows.

## Development

```sh
cargo build --release
cargo test            # scene geometry and timer behaviour
cargo install --path .
```

```
src/main.rs      entry, event loop, key and mouse routing
src/app.rs       state, focus, per-pane input
src/project.rs   manuscript tree, frontmatter, scaffolding
src/editor.rs    word-wrapping buffer with visual cursor movement
src/scene.rs     the forest and the pomodoro
src/theme.rs     presets, custom palette, load/save
src/ui.rs        rendering
src/music.rs     youtube-music API Server client
site/            the landing page
```

### Releasing

Bump `version` in `Cargo.toml`, add a matching section to `CHANGELOG.md`,
commit, then tag and push the tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

The **Release** workflow ([dist](https://github.com/axodotdev/cargo-dist))
builds every platform and publishes the GitHub release with both installers;
**installer-check** then installs it with the one-liners on Windows, macOS and
Linux and runs it. Nothing is published unless every build succeeds.

Issues and pull requests welcome.

## License

MIT © Josh King
