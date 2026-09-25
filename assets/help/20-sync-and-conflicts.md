# Sync & conflicts

A book is a folder, so any sync service carries it: Dropbox, Google Drive, pCloud, Box, OneDrive, iCloud Drive. Grimoire makes sure none of them can cost you words.

## Two devices change one scene

**Nothing is ever written over.** Before every save Grimoire checks the file on disk. If it changed somewhere else, the other version stays the scene, and your words from here are kept beside it as a *parked copy*.

Sync services make their own copies when two devices clash — Dropbox's "conflicted copy", pCloud's "(conflicted)", Box's "(1)", and so on. Grimoire recognises all of them. They appear right under their scene, marked with where they came from ("Dropbox copy"), and they're never counted or exported twice. They move with their scene.

While there are any, the status bar says `⚠ 1 conflict`.

## Settling a conflict

*Settle conflicts…* (menu or `Ctrl-K`) lists every copy. Choose one and it opens beside its scene, with how much longer or shorter it is. Then:

- `1` **take the copy** — the scene's own words go into its history first;
- `2` **keep the scene** — the copy goes to the Trash;
- `3` **keep both** — the copy becomes a scene of its own, right after the original.

One `Ctrl-Z` takes it back. Nothing is ever deleted.

## Files that aren't here yet

A file your sync service hasn't downloaded (online-only, still arriving, or offline) is shown in the outline as *offline* and is never saved over. Grimoire reads it once it arrives. Export waits until every scene can be read.

## Files that blink

Some services replace a file by removing it for a moment. A scene that vanishes briefly isn't treated as deleted, and a file still being written isn't read until it settles.

## Names

Everything Grimoire names is safe on every service and computer: no characters Windows forbids, nothing too long, and never two names that differ only by capitals or accents (on a Mac, on Windows and in Box, `Wren.md` and `wren.md` are the same file). If two such files are already in a folder, both are shown marked *name clash* for you to rename.

## Good habits

- Keep the book's folder available offline ("Always keep on this device", "Make available offline").
- Don't put one book in two sync services at once.
- Let the service finish syncing before you open the book on another device.

At launch the status bar says which service the book is in.
