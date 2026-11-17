# Working on Grimoire

## The rule: what the app can do, the help says

Grimoire's help lives in the app (`assets/help/*.md`, opened with `F1` or `?`).
A writer who can't find a feature in the help doesn't have that feature. So a
change a writer can see is not finished until the help says it.

**Every change that adds, removes or alters something a writer can see or
press updates the help in the same commit.** That means a key, a menu row, a
box, a message's meaning, a default, where files go, a command or a flag.

Which topic:

| The change | The help that must say so |
|---|---|
| A menu row (any group) | `27-the-menu.md`, in its group, and the topic for the feature |
| A key | `24-every-key.md`, and the topic for the pane or box it works in |
| A `Ctrl-K` action | Any topic, by the action's own name |
| A command or flag (`grimoire …`) | The topic for the feature, and `grimoire --help` in `src/main.rs` |
| A new box or screen | The topic for the feature, and its topic in `help_context` (`src/app/overlays/help.rs`) so `F1` opens the right page |
| Where a book or a file is kept | `26-your-books.md` and `22-on-disk.md` |
| A setting | `23-settings.md` |
| A changed behaviour (what `Enter` does, what's undone) | Wherever the help describes the old behaviour: search for it |

Before calling a change done, search the help for the old behaviour
(`grep -rn "<the old words>" assets/help`) and correct every place that
still describes it.

A feature release also updates, in this order: `CHANGELOG.md` (it becomes the
release notes), `README.md`, and the site (`site/src/pages/index.astro`, and
the curriculum under `site/src/pages/learn/` when it touches what a class
does). Then the site is built and deployed.

### Tests that hold the rule

These fail when the help falls behind. They are in `src/ui_tests.rs`:

- `every_menu_row_is_in_the_menu_help`: every menu row is named in
  `27-the-menu.md`.
- `every_palette_action_is_explained_in_the_help`: every `Ctrl-K` action is
  named somewhere in the help.
- `every_command_is_explained_in_the_help`: every `grimoire <command>` is
  mentioned in a topic.
- `every_key_in_every_key_does_something`: every key in a table of
  `24-every-key.md` is pressed and must do something, and has a row in
  `EVERY_KEY`. Only real, pressable keys go in code marks inside those tables.
- `each_place_opens_the_help_on_its_own_topic` and
  `every_topic_fits_and_scrolls_at_80x24_and_120x35`.

A test passing is the floor, not the job. The tests check that a name
appears; only a person (or you) can check that the page explains it well.

## How the help is written

Casual, warm, plain. Short sentences. No em dashes. Menu paths use `›`
(*Esc › This book › Save a copy*). Say what a thing does for the writer
before how it works. Each topic stands on its own and points to its
neighbours by name (see *Your books*).

## Before a commit

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Pipe none of these into another command when the result decides a commit: a
pipe hides the exit code.
