# Paperback & EPUB

The same book compiles for readers as well as agents. Both come from *Export…*, and both go through the same compile as the manuscript: no empty chapters, curly quotes and real dashes, TKs listed first.

## Paperback

Tick **Paperback** in the export dialog; `←` `→` on its row choose the trim size: 5×8, 5.25×8, 5.5×8.5 or 6×9 inches.

- Mirrored margins, with the inside margin KDP asks for at the book's page count.
- The author's name on left-hand pages and the title on right-hand ones, none on chapter openings.
- Every chapter opens on a right-hand page, a third of the way down, with a small-caps lead-in (or a drop cap).
- Justified, hyphenated text; an ornament between scenes; a title page and a copyright page.

You get a Word file and, with LibreOffice installed, a print-ready PDF. `[paperback]` in `novel.toml` sets the font, size, the scene ornament and drop caps.

## EPUB

For phones and e-readers.

- **A cover**: put `cover.jpg` (or `cover.png`) in the book's folder.
- **A contents page**, as Kindle and others ask for.
- **Details** from `[ebook]` in `novel.toml`: language, ISBN, publisher, description, subjects, series and number, rights, publication date.

## Front and back matter

The **Front Matter** section holds pages that go before the story — a title page, copyright, dedication — in folders for each edition: *Manuscript Format*, *Paperback* and *Ebook*. **Back Matter** holds pages after it — Acknowledgements, About the Author, Also By — in *Paperback* and *Ebook* folders. A page loose in the section goes into every edition, and a page marked `compile: false` stays out until you're ready.
