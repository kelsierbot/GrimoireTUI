# On disk

A book is an ordinary folder you can open, copy and back up without Grimoire.

## The folder

| Path | What it is |
|---|---|
| `novel.toml` | the title, author, word targets, and export details |
| `Novel-Format.md` | the guide to how the book is laid out |
| `manuscript/` | the story: parts, chapters and scenes |
| `characters/`, `places/` | the notebook |
| `notes/`, `research/` | more of the notebook |
| `front-matter/`, `back-matter/` | pages before and after the story, by edition |
| `template-sheets/` | Character and Setting sketches to copy |
| `dictionary.txt` | words spellcheck accepts in this book |
| `cover.jpg` | the EPUB's cover, if you add one |
| `exports/` | what Export writes |
| `project.md` | a map of the book, if you ask for one |
| `.grimoire/` | history, recovery, the Trash, and where you left off |

Folders are containers, at any depth, and `.md` files are scenes or notes. A leading number like `01-` sets the order and is hidden in the outline.

## Two files made on request

Both are in `Ctrl-K`:

- *Update project map* writes `project.md`: every part, chapter and scene, with word counts and a link to each file, in a marked block Grimoire rewrites each time. Anything you write above or below the block is left alone.
- *Compile manuscript (Markdown)* writes the whole book as one Markdown file, `title-manuscript.md` beside `novel.toml`, through the same compile as Export.

## A scene's frontmatter

The lines between `---` at the top of a scene:

- `title` — its name in the outline.
- `pov`, `status`, `synopsis` — shown on the corkboard. Statuses are idea, outline, draft, revised and done.
- `target` — the scene's word target.
- `compile: false` — keep it in the outline but out of exports.

A note can have `aliases` too. Grimoire only changes the one line it means to, so Obsidian and anything else can keep fields of their own there.

## novel.toml

`title`, `author`, `target_words`, `daily_target`, and `part_label` (what the book calls its parts: Part, Act, Book). Below them, tables Grimoire writes for you: `[contact]` (author details), `[manuscript]` (the manuscript's look), and optionally `[ebook]` and `[paperback]`.

## Your settings

Kept in `~/.config/grimoire/`: `settings.toml`, `theme.toml`, `music.toml`, and your every-book spelling list, `dictionary.txt`.
