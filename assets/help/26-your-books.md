# Your books

Every book lives in a folder of its own, and you can have as many as you like. Starting one, or hopping between them, is a couple of keys.

## Starting a new book

*Esc › Start a new book…* (or type *new book* in `Ctrl-K`).

Type a name and press `Enter`. That's all. The book you had open is saved and put away first, and the new one opens on a page that shows how a book is laid out. It comes with parts, chapters and scenes already in place, ready to fill in, rename or delete.

A plain name goes in a folder beside the book you had open. With *~/Documents/Grimoire* open, *The Long Night* becomes *~/Documents/The Long Night*. The box shows exactly where before you press `Enter`, and a path (like `~/Writing/The Long Night`) puts it anywhere you like.

If there's already a book by that name, `Enter` opens it instead of making a second one. A folder that already has other things in it is left alone, so try another name.

From a terminal, `grimoire new <folder>` starts one too.

Got a draft already? See *Bringing in a draft*, just below.

## Bringing in a draft

Already have a draft somewhere else? Bring it in and it becomes a Grimoire book, split up and ready to work on. Grimoire reads a **Word file** (`.docx`), **Markdown** (`.md`) or **plain text** (`.txt`). From Google Docs, *File › Download › Microsoft Word* first.

*Esc › Open another book… › Bring in a draft…*, or type a draft's path into *Start a new book…*. You can type the path, or drag the file into the terminal window. `Ctrl-K` › *Bring in a draft* works too, and from a terminal it's `grimoire import <file>`.

Here's how it's split up:

- **Chapters** start at headings. In Word that's the *Heading 1* (or *Heading 2*) style; in Markdown it's `#` or `##`. With no headings at all, a line on its own like *Chapter Three* or *Prologue* counts.
- **Parts** are headings that start with *Part*, *Book* or *Act* and a number, like *Part Two*.
- **Scenes** start at a scene break: `***`, `* * *`, `#` or `~` on a line of its own. A heading one level below the chapters starts a titled scene.
- *Italics* and **bold** come across.
- Anything before the first chapter, like a title page, is kept in **Notes**, so it doesn't turn into a chapter.
- An *END* on its own at the very end is left off, since Grimoire's export adds its own.

The draft itself is never changed. The new book goes beside the one you had open, named after the draft, and opens straight away. It tells you how many chapters, scenes and words came across, and every word lands somewhere.

## Switching books

*Esc › Open another book…* lists the books you've had open on this computer, newest first. Pick one and press `Enter`: the book you're in is saved and closed, and the other opens right where you left off.

- *A book in another folder…* opens one that isn't on the list yet. Type its folder.
- In `Ctrl-K`, type part of a book's name. Every book on the list is there as *Open the book …*.

Your music keeps playing, the Pomodoro keeps counting, and the theme stays just as it is.

## Which book opens when Grimoire starts

`grimoire` on its own opens the book in the folder you're in, if you're in one. Otherwise it opens the last book you had open, so after a switch that's the one you switched to.
