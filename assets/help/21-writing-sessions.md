# Writing sessions

Writing sessions keep a snapshot of the book each time you sit down to write, labelled like a diary — *Tuesday evening · Act Two · 1,240 words* — so you can see what changed in any sitting. It needs Git installed.

## Turning it on

*Writing sessions* in `Ctrl-K` offers to turn it on for this book (`y`). From then on, each session is kept when you quit, or straight away with *Save this session now*.

## Looking back

*Writing sessions* lists every session, newest first.

- `Enter` (or `→`) shows the scenes that session changed; `Enter` on one shows the words that changed.
- `←` or `Esc` goes back; `s` saves a session now; `Esc` closes.

## Where it's kept

The history is kept on **this computer**, outside the book's folder, where no sync service can damage it: in Grimoire's data folder (`~/.local/share/grimoire` on Linux, `~/Library/Application Support/grimoire` on a Mac, `%APPDATA%\grimoire` on Windows). Each computer keeps its own.

If that history has a Git remote, sessions are sent to it in the background, so they're backed up off the computer too.

## Older books

A book from an earlier version may keep its history in a `.git` folder inside the book. That keeps working — but if the book is in a sync service, the menu offers **Move writing history out…** (in `Ctrl-K`, *Move writing history out of the synced folder*), which copies it to the data folder, checks the copy, and moves the old one to the Trash. Your scenes are never touched.
