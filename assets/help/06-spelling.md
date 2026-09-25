# Spelling

Misspellings are underlined as you write, from a built-in English dictionary. Names from your notebook are never flagged, and words inside `%% notes %%` are left alone.

## Fixing a word

- **Click** an underlined word, or **right-click** it. Its suggestions appear right under it, in the word's own capitals ("Teh" offers "The").
- Click a suggestion, or press its number, or move with `↑` `↓` and press `Enter`.
- Anything else you type goes on to the prose as usual, so clicking a word to fix it by hand never gets in the way.
- `F8` (*Spelling suggestions for this word* in `Ctrl-K`) does the same from the keyboard: suggestions for the word at the cursor, or for the next misspelling after it. `F8` again moves on to the next one.

A fix is a single step to undo with `Ctrl-Z`.

## Ignoring a word

Under the suggestions are three ways to stop a word being called a mistake:

- **Ignore for now**: until you close Grimoire.
- **Always ignore — this book**: added to `dictionary.txt` in the book's folder, so it travels with the book.
- **Always ignore — every book**: added to your own list in `~/.config/grimoire/dictionary.txt`.

The underline disappears everywhere the word appears, straight away.

Spellcheck can be turned off and on in *Esc › Settings*.
