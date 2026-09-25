# Changelog

## 0.5.6 — grimoiretui.com

- Grimoire has its own home now: **grimoiretui.com**. *About Grimoire…*
  and the installers point there, and the old address forwards to it.

## 0.5.5 — The license, in plain words

- **License** is a new help topic: Grimoire is MIT, so commercial use is
  welcome, credit is required (keep the copyright notice and license with
  it), your books are entirely yours, and a donation is appreciated but never
  required. `l` in *About Grimoire…* opens it, and so does `Ctrl-K` ›
  *License*.
- *About Grimoire…* says the same in two lines.

## 0.5.4 — Help, About and Donate

- **Help inside the app**: 24 topics explaining every feature, from getting
  started to compiling and sync. `F1` opens it anywhere, `?` wherever you
  aren't typing, or *Help…* in the menu and `Ctrl-K`. It opens on the topic
  for where you are, and typing searches every topic.
- `F1` now opens Help; `Esc` is the menu.
- **About Grimoire…** says which version you're running and that it's made by
  Catfinity Studios, with a link to catfinity.com. `grimoire --version` says
  the same.
- **Donate…**: Grimoire is free and a donation is never required, but it's
  appreciated and goes toward Grimoire's development — the box opens the
  Ko-fi page.
- Writing sessions: `l` opens a session's changes, like `→`; `h` no longer
  does.

## 0.5.3

- **The Rainbow theme's colours flow**: the spectrum across its borders, titles
  and progress bar glides round the wheel every twelve seconds (it used to
  drift so slowly it looked still).
- Fixed on a Mac: a scene with an accented name listed each saved version
  twice in its history.

## 0.5.2 — a paperback and a proper ebook

- **Paperback** (Export ▸ Paperback, or `grimoire export --paperback --trim
  6x9`): a print-ready interior — a DOCX for Word, and a PDF when LibreOffice
  is installed. Trim 5×8, 5.25×8, 5.5×8.5 or 6×9, mirrored margins with KDP's
  gutter for the page count, author and title running heads, every chapter on
  a right-hand page with a small-caps lead-in (or a drop cap), justified and
  hyphenated text, an ornament between scenes, a title and copyright page.
  `[paperback]` in `novel.toml` sets the font, size, ornament and drop caps.
- **EPUB**: a cover (`cover.jpg` in the book), a contents page, `[ebook]`
  metadata (language, ISBN, publisher, description, subjects, series) and back
  matter. EPUBCheck-clean.
- **Back Matter**: a new section for Acknowledgements, About the Author and
  Also By, per edition like Front Matter.
- The paperback and the ebook are compiled exactly like the manuscript: no
  empty chapters, curly quotes and real dashes, TKs kept and warned about, a
  Prologue's own heading, samples.

## 0.5.1 — compiled for submission

- **Export compiles a submission manuscript in standard manuscript format**
  — William Shunn's modern standard: a title page with your contact block,
  byline and rounded word count; "Surname / KEYWORD / page" on every page
  after it; parts and chapters a third of the way down, a part sharing its
  first chapter's page; `#` between scenes; END.
- **Manuscript look…**: Modern (Times) or Classic (Courier, underlined
  italics, straight quotes), Letter or A4, double or 1.5 spacing, five
  chapter-heading styles, flush or indented first paragraphs, END / THE END /
  none, and the header keyword.
- **Author details…**: legal name, byline, address, phone, email and agent,
  for the title page.
- **Samples for agents**: chapters X–Y, the first N chapters, or whole scenes
  up to N words — the title page still gives the whole book's count.
- **PDF**, through LibreOffice when it's installed.
- **Typography in every export**: curly quotes and apostrophes, real dashes and
  ellipses — your files are left as you typed them.
- **Nothing blank**: empty scenes, chapters and parts are left out, and `#`
  only ever sits between two written scenes.
- **TKs stay visible** — they were dropped mid-sentence — and export lists
  where they are first.
- **A Prologue or Epilogue keeps its own heading.**
- Fixed: reordering sections could reduce an unreadable `novel.toml` to one
  line.

## 0.5.0 — safe in every sync service

A complete pass over how a book behaves in pCloud, Google Drive, Dropbox and
Box (and OneDrive, iCloud Drive, Syncthing), checked against each service's
documented behaviour and tested on a real pCloud Drive.

- **Conflict copies from every sync service are recognised** — Dropbox's
  "conflicted copy", pCloud's "(conflicted)", Box's "(1)", OneDrive's device
  suffix, iCloud's "2", Syncthing's "sync-conflict", Google Drive's variants,
  and Grimoire's own — named in the tree ("Dropbox copy"), never counted or
  exported twice, and moved with their scene. `⚠ N conflicts` shows in the
  status bar until you settle them.
- **Settle conflicts…** puts both versions side by side: take the copy, keep
  the scene, or keep both. One `Ctrl-Z` undoes it; nothing is ever deleted.
- **A file your sync service hasn't downloaded, or can't deliver offline,
  never stops a book opening.** It's shown as *offline*, never saved over, and
  read once it arrives. An online-only placeholder (empty, or iCloud's stub) is
  never taken for a scene cut to nothing or deleted.
- **A file that blinks** while a service replaces it, or that is still
  arriving, is no longer sent to the trash or taken in half-written.
- **Saves never write a file in place**: a temp name every service ignores,
  retried while another program holds the file, and the words kept in
  recovery until it succeeds.
- **Every name Grimoire makes is safe on every service and system** — no
  characters Windows forbids, no reserved names, nothing too long, never two
  names that differ only by capitals or accents. Pairs like that already on
  disk are marked *name clash*.
- **Writing-session history lives on each machine, outside the book**, where
  no sync service can damage it; an older book's history can be moved out.
- **The phone never saves over words** that arrive just after its own save,
  and moving its shelf never loses a file that changes mid-move.
- Grimoire says which sync service a book is in.
- Fixed: an unreadable `novel.toml` could be replaced by a one-line file at
  launch; a scene that became unreadable could spawn a conflict copy every two
  seconds.

## 0.4.7

- **Custom themes mix and match**: under the swatches, choose any theme's
  Pomodoro world and any theme's Visualizer (`←` `→`), previewed live in the
  pane — Kanagawa's great wave with Synthwave '84's neon grid, say.
- **Rainbow's Pomodoro is redrawn**: five fine ribbons curving in braille,
  standing in two soft clouds, a light running slowly along the outer band by
  day — instead of chunky blocks.
- **Tokyo Night's Visualizer is a neon city**: night-blue towers with warm and
  cool windows set into them, a neon roofline on each, a mast with a slow red
  light on the tallest, neon smeared on the wet street.
- The spelling popup is wide enough for its own hint.
- **The README has screenshots**, drawn by Grimoire itself.

## 0.4.6

- **The garden is gone.** The pane under the tree has two views, the
  **Pomodoro** and the **Visualizer**; `←` `→` switches between them.
- **Every theme has its own Pomodoro world**: a wizard's tower for Grimoire, a
  castle and bats for Dracula, snowy peaks and the aurora for Nord, cats on a
  fence for Catppuccin, the great wave for Kanagawa, a neon horizon for
  Synthwave '84 (its striped sun is the clock), a rainbow over the hills for
  Rainbow — nineteen in all. Lost Forest and custom themes keep the glade.
- **The ground fills in behind the sun** as a session goes, and a paused sun
  waits, dimmed.
- **The Visualizer looks different in every theme** too — meters, beads,
  dotted frost, a lit skyline, a synthwave grid, a rolling wave, a mirrored
  prism — and custom themes keep the original.
- **It always names the song**, cut to fit, in its title or written along the
  bottom and lit as the song plays.
- **Calmer**: a gentler beat, fewer stray peak marks, and a quiet resting line
  with the reason when nothing's playing.

## 0.4.5

- **Volume steps evenly**: each `+` or `-` moves it about ten (100 → 90 → 80),
  where YouTube Music used to lurch 100 → 74 → 55 because the step was taken
  on the scale it accepts rather than the one it shows.
- **Below full volume, the music pane shows the level** (`70%`) beside the time.

## 0.4.4

- **Click a misspelt word** (or right-click it) for its fixes, right under the
  word, in the word's own capitals — or tell Grimoire to stop calling it one:
  for now, always in this book, or always in every book. Typing anything else
  just carries on writing.
- A spelling fix undoes with one `Ctrl-Z`.

## 0.4.3

- **The music pane takes every one of the player's keys** when it's
  highlighted: `[` `]` change track, `←` `→` seek ten seconds, `space`, `r`
  repeat, `s` shuffle, `+` `-` volume, `l` like. Its edge names them while it
  has focus.
- **Repeat, shuffle, volume and like are on screen**: under the player's
  progress bar, and as `↻` `↻1` `⇄` `♥` beside the pane's time. Before, `r` and
  friends worked but nothing showed it, so they looked broken.
- **Every key says what it did** — "repeat: one", "shuffle on", "volume 100 —
  as loud as it goes" — read back from the player once it has applied the
  change, and a command the player refuses now says why instead of vanishing.

## 0.4.2

- **A spellbook heads the tree**: an open grimoire, light rising from its
  spine, sparkles twinkling above it — in your theme's colours, and in every
  colour under Rainbow. It steps aside on short windows so the outline keeps
  its rows.
- **Focus mode says where it is**: `^D focus mode` sits right after `Esc menu`
  in the status bar, and the idle timer's title reads `F2 timer · ^D focus
  mode`. The running timer now says `writing · 18:04` rather than `focus`, so
  "focus" only ever means focus mode.

## 0.4.1

- **Ten new themes**: Rosé Pine, One Dark, Monokai, Kanagawa, Ayu Mirage,
  Night Owl, Material Palenight, Synthwave '84, GitHub Dark — and **Rainbow**,
  where everything drawn in the accent (focused borders, titles, the progress
  bar, the word count) is a slowly drifting spectrum, the spectrum view's bars
  climb from red to violet, and the clearing is every colour at once.
- **The theme picker shows each theme's colours** beside its name, and scrolls
  on a short terminal.

## 0.4.0

### Keeping every word

- **Changes made elsewhere are never overwritten.** If Obsidian, a sync app or
  the phone changes a scene while Grimoire is open, Grimoire shows the new
  version; if you had unsaved words in it too, they're kept beside it as
  "Scene (from bazzite, …)", marked *parked copy* in the tree. Scenes added,
  moved or deleted outside Grimoire show up by themselves; a deleted scene is
  never brought back, and any unsaved words from it go to the trash.
- **One file that isn't UTF-8 no longer stops a book opening.** It's shown
  read-only and never rewritten.
- **The recovered-words box never throws words away on a keystroke:** choose
  with ↑↓ and `Enter`. "Keep the saved versions" puts the recovered words in
  the trash rather than deleting them.
- **Moving a section right after typing** no longer loses the last few
  seconds of writing.
- **Two things with the same name deleted in the same second** both stay in
  the trash.
- **A move that can't finish** (a locked folder, a sync app holding a file)
  puts everything back, and anything a crash left mid-move reappears in the
  tree next time the book opens.
- **Undoing a move keeps its link fixes**, and a new scene no longer inherits
  a deleted scene's undo or history.
- **Find and replace match whole words in their exact case** by default —
  `Ctrl-W` for inside words, `Ctrl-E` for any case — and one `Ctrl-Z` undoes a
  replace across the whole book.

### Writing

- **Focus mode** (`Ctrl-D`): just your prose, centred, with a quiet status
  line. `Esc` still opens the menu; `Ctrl-D` again brings everything back.
- **A readable line length**: prose wraps at 72 columns, centred. *Line
  width* in Settings changes it (60 / 72 / 80 / 100 / full).
- **Typewriter scrolling**: in focus mode the line you're writing stays
  mid-screen; elsewhere you're never drafting on the bottom row.
- **Notes that never reach the book**: `%% like this %%` (Obsidian's comment
  syntax, may cross lines) and TK — dimmed in the editor, left out of every
  word count, compile and export, kept on disk exactly as typed. **`Ctrl-T`**
  lists every note and TK in the book; type to filter, `Enter` goes there.
- **Sprints**: *Start a sprint…* sets a word goal and minutes, starts the
  timer, and the status bar counts `sprint 212/500 · 14:32`. A scene's
  `target:` shows as progress in the editor's title.
- **Revision passes**: *Next scene still in draft*, and *Echo words* (the same
  word again within forty).
- **A scene beside**: `v` in the tree shows another scene, read-only, next to
  the one you're writing. `Esc` closes it; `Enter` switches to writing in it.
- **An empty scene says how to start.**

### Around the desk

- **The Esc menu has the new tools**: focus mode, a scene beside, notes &
  TKs, next scene in draft, echo words and sprints; Settings has line width
  and typewriter scrolling. Rows that can't do anything right now are hidden,
  and the menu scrolls on a short terminal. From the editor, the menu's
  New/Rename/Delete act on the scene you're writing, as `Ctrl-K`'s always did.
  Lists answer PgUp/PgDn and Home/End throughout.
- **Export… is in the menu** — Word, EPUB or Markdown from one dialog, which
  now fits an 80-column terminal. Compile and the project map are still in
  `Ctrl-K`.
- **A new book opens on its Novel Format guide**, written as plain prose.
- **New books count in parts again** (Part One, `p` new part). Older books
  keep their folders' names — a book an earlier version switched to pages
  keeps its pages, and one with its own label (Act, Book) is untouched.
- **The spectrum works on Linux**: it hears whatever the machine plays through
  PipeWire (`pw-record`) or PulseAudio (`parec`), only while the view is
  showing; with neither installed it says so instead of sitting blank.
  `GRIMOIRE_MONITOR=<sink>` listens to a different output.
- The menu leaves out *Music player* while music is off; history compares each
  version with now ("6 words shorter"); resuming an empty scene says it was
  left open; corkboard cards without a synopsis show the scene's first line; a
  character note's title names its section.

### Under the hood

- The book's logic lives in `grimoire-core`, shared with the phone app, which
  now builds from the same repository.
- Tests run on Linux, macOS and Windows for every change, with clippy and
  rustfmt enforced.

## 0.3.6

- **`Esc` closes an open character note** whichever pane you're in — 0.3.5
  promised this but only did it with the note itself focused; from the editor
  it opened the menu and left the note up. `Ctrl-O` on the same name (or on
  none) now puts the note away too.
- **The status bar leads with what `Esc` does**, so an 80-column terminal no
  longer cuts "Esc menu" off the end.

## 0.3.5

- **`Esc` opens the menu**, from any pane — no more reaching for `F1`, which
  many keyboards only send with `Fn` held (it still works). `Esc` again closes
  it and puts you back where you were, so it no longer drops you from the
  editor into the tree; `Shift-Tab` does that. A selection in the editor, or
  an open note beside it, is closed by the first `Esc`.
- **Quit Grimoire** is in the menu, at the bottom. It saves everything first,
  like `Ctrl-Q`.

## 0.3.4

- **"today" always moves when you write.** It's the day's net change in the
  manuscript, shown below zero (`today −29`) after cutting more than you've
  written, instead of sitting at 0 until you'd written the cut words back.
  Deleting a scene or bringing it back with `Ctrl-Z` doesn't change it, cut and
  paste within the book comes out even, and it starts over at midnight even if
  Grimoire stays open.

## 0.3.3

- **Closing the terminal window really closes Grimoire.** It used to be left
  running, stuck at full CPU on the closed terminal (a crossterm reader that
  never returns), without the save-on-close ever happening. Keys are now read on
  their own thread, so closing the window saves and exits in a moment.

## 0.3.2

- **Everything in the tree moves**: drag a section, or press `K`/`J` on it, to
  reorder the sections (saved in `novel.toml`; the trash stays last). Notes
  without a number in their filename can be moved too — the folder is numbered
  on the first move. All of it undoes with `Ctrl-Z`.
- **A shorter menu**: themes, music source, music, spellcheck and tree icons
  are in `F1` → **Settings…**, which opens a menu of its own.

## 0.3.1

- **Undo in the tree**: `Ctrl-Z` outside the editor takes back a delete,
  rename, move (a whole drag at once) or new item; `Ctrl-Y` does it again. A
  deleted item comes back out of the trash to where it was. Typing is still
  undone from the editor.
- **Tree icons are optional** and start off: F1 → *Turn tree icons on*.

## 0.3.0

- **A tree in sections**: Novel Format, Manuscript, Characters, Places, Front
  Matter, Notes, Research, Template Sheets and Trash, each with its own symbol,
  every section and folder foldable, headed by the book's title.
- **Pages** are the book's largest division (they were parts): `p page`.
- **Front matter by edition** — Manuscript Format, Paperback, Ebook — with
  starter title, copyright and dedication pages, left out of exports until
  you include them. The EPUB takes the Ebook pages; Word takes the Manuscript
  Format ones.
- **Template Sheets** for a character and a setting, and a Novel Format guide.
- **Older books are upgraded once, on opening**: notebook folders move into
  their sections, parts become pages, links follow.
- Popup keys stand out in the accent colour; the clearing pane has a view
  switcher (`◂ clearing spectrum garden ▸`); the garden is titled.

## 0.2.0

Nothing written is one keystroke from gone, and a hundred scenes are easy to
find and shape.

- **Autosave**, two seconds after you stop typing, through temporary files so
  a crash can't tear a scene; closing the window or quitting saves first.
- **Recovery**: words that couldn't be saved are offered back next launch.
- **Undo and redo**, a word at a time; typing replaces a selection; paste is
  one step; Ctrl-X cuts.
- **Scene history** (`H`): dated versions with a word diff; restore any.
- **Command palette** (`Ctrl-K`): every action, scene, note and theme by name.
- **Find and replace** in a scene (`Ctrl-F`) or across the book (`/`), with a
  confirm before replacing across scenes.
- **Name drift**: near-miss spellings of notebook names, fixed in one go.
- **Spellcheck** with a bundled en_US dictionary that knows your notebook's
  names; `F8` for suggestions; words you add go in `dictionary.txt`.
- **Move scenes and chapters** (`Alt-↑/↓`, `K`/`J`, or drag in the tree),
  across chapters and acts, with `[[links]]` kept pointing at the right files.
- **The corkboard** (`b`): index cards with POV colours, status, synopsis and
  words against target; filter by POV; edit on the card.
- **The codex** (`Ctrl-O`): a notebook note beside the scene, with every scene
  that mentions it.
- **Export for readers**: Word (standard manuscript format), EPUB and Markdown,
  whole book or chosen acts; also `grimoire export`.
- **Resume** where you left off, on any computer the book reaches.
- **Writing sessions**: optional Git history labelled like a diary, with
  background backup to a remote.
- **Music is off by default**; turn it on from the menu or palette.

## 0.1.0

The first release, and the first with ready-to-run downloads — no Rust
toolchain needed.

Built for Windows (x64), macOS (Apple Silicon and Intel) and Linux (x86_64 and
ARM64); the one-line installers are below.

**What's in it**

- A manuscript that is just folders and Markdown files — the directory tree is
  the outline. New books start as three acts of nine chapters with three scenes
  each, plus a notebook and a visible trash.
- Create, rename and delete parts, chapters and scenes from the tree; deleting
  moves things to the trash rather than destroying them.
- Word counts and daily progress, a project map, and `grimoire compile` to
  assemble the manuscript.
- Music while you write: YouTube Music, Spotify (macOS and Linux), Jellyfin and
  Plex. `grimoire music-setup youtube-music` installs and connects YouTube Music
  on all three systems.
- Themes, switched with `F9`.
