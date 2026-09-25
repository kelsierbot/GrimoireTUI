# The outline

The outline is the book's folders and files, shown as a tree. Every change you make here is a change on disk: renaming a scene renames its file.

## Moving around

- `↑` `↓` or `j` `k` move.
- `→` or `l` unfolds; on a scene it opens it. `←` or `h` folds, or jumps up to the folder above.
- `Enter` or `Space` folds and unfolds a folder, or opens a scene.

## Making things

- `n` a new **scene**: at the end of the chapter you're in, or in a notebook section, a new note there.
- `c` a new **chapter**, named for you ("Chapter Four"), at the end of the part you're in.
- `p` a new **part**, at the end of the manuscript. If your book calls its parts something else, like Acts, `part_label` in `novel.toml` says so and Grimoire uses that word everywhere.
- `N` a new **folder** beside the one selected.

Each asks for a name, with a suggestion already filled in. Type to replace it, or press `Enter` to take it. Nothing is ever renumbered to make room: a new item goes at the end.

## Renaming and deleting

- `r` renames what's selected. The file is renamed to match, and keeps its number and place.
- `d` deletes it. Grimoire asks first, by name, and moves it to the **Trash** section at the bottom of the outline.
- Deleting something that is **already in the Trash** removes it for good. That one asks too.

## Moving things

- `K` and `J`, or `Alt-↑` and `Alt-↓`, move the selected scene or folder up and down (*Move up* and *Move down* in `Ctrl-K`). At the ends of a chapter a scene crosses into the next one.
- You can also **drag** a row with the mouse.
- On a section heading (Characters, Places…), the same keys change the order of the sections.
- `[[links]]` in your notes follow a scene when it moves.

## Taking it back

`Ctrl-Z` in the outline undoes the last delete, rename, move or new item; `Ctrl-Y` redoes it. A deleted scene comes back out of the Trash to exactly where it was.

## Other keys here

- `H` the selected scene's **history**.
- `b` the **corkboard**.
- `v` shows the selected scene **beside** the one you're writing.
- `/` finds in the whole book.
- `t` or `F9` changes the theme.
