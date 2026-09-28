# Writing

Open a scene from the outline and press `Tab` (or click the page) to write in it.

## Moving

- The arrow keys move the cursor. `Up` and `Down` move one **line on the screen**, not one paragraph, so they behave the way you'd expect inside a long wrapped paragraph.
- `Home` and `End` go to the start and end of the line; `PgUp` and `PgDn` move a screenful.
- Click anywhere in the prose to put the cursor there.

## Selecting, cutting and pasting

- **Drag** with the mouse to select.
- `Ctrl-C` copies the selection and `Ctrl-X` cuts it. `Ctrl-V` pastes (so does your terminal's own paste, `Ctrl-Shift-V` or `Cmd-V`); a paste is one step to undo.
- `Ctrl-A` selects the whole scene (*Select the whole scene* in `Ctrl-K`).

## Italics and bold

- `Ctrl-I` puts **italics** on the selected words, or on the word the cursor is in. `Ctrl-I` again takes them off.
- `Ctrl-B` does the same with **bold**.
- Both are in *Esc › Writing tools* too, and in `Ctrl-K` (*Italic*, *Bold*). Some terminals send `Ctrl-I` as `Tab`; the menu always works.

In the file, italics are `*stars*` and bold is `**double stars**`, the way Markdown writes them, and every export turns them into real italics and bold.
- Typing over a selection replaces it. `Backspace` or `Delete` removes it.
- `Esc` lets go of a selection. With nothing selected, `Esc` opens the menu.

## Undo

`Ctrl-Z` undoes, a word at a time, and `Ctrl-Y` (or `Ctrl-Shift-Z`) redoes. Each scene keeps its own undo, even after you switch to another scene and back.

## Saving

A scene is saved two seconds after you stop typing. The status bar shows `○ saving` while a change waits and a brief `✓ saved` once it's written. If a save ever fails, the words go to recovery and Grimoire keeps trying.

## Useful while writing

- `Ctrl-D` focus mode: just the prose.
- `Ctrl-T` every note and TK in the book.
- `F8` spelling suggestions, or click an underlined word.
- `Ctrl-O` on a name opens its note from the notebook.
- `Ctrl-F` finds in the scene.
- The page's title shows where you are, like `Chapter One / Scene Two`, and the scene's progress when it has a word target.
