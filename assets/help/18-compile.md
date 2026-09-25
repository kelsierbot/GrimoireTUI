# Compile: the manuscript

When the book is ready to send, *Export…* in the menu (*Export for readers* in `Ctrl-K`) compiles it into a manuscript in **William Shunn's modern standard manuscript format** — the one agents and editors expect.

## What you get

- **A title page**: your contact block top left, "about 82,000 words" top right (rounded the way publishers expect), the title halfway down, and your byline under it.
- **Every page after it**: Times New Roman 12, double-spaced, one-inch margins, half-inch indents, and `Surname / KEYWORD / page` in the top right corner.
- **Chapters** start a third of the way down a new page; a part shares its first chapter's page. A Prologue or Epilogue keeps its own heading.
- **Scene breaks** are a centred `#`, and the manuscript ends with END.
- **Typography**: curly quotes and apostrophes, real dashes and ellipses. Your own files stay exactly as you typed them.
- **Nothing half-finished goes out quietly**: empty scenes and chapters are left out, `%% notes %%` are removed, and every TK is listed before anything is written.

## The export dialog

- `↑` `↓` move between rows. `Space` or `Enter` ticks a format or a part.
- **Formats**: Word manuscript, PDF, Paperback, EPUB and Markdown.
- **Include**: which parts go in.
- **How much**: `←` `→` choose the whole book, the first chapters, chapters X–Y, or the first N words — the samples agents ask for — and type the number or range. The title page still gives the whole book's word count.
- **Look…** and **Author details…** (below).
- `x` (or `Enter` on the Export button) exports into the book's `exports/` folder. Files are named the way agents ask: `Surname_Title_Manuscript.docx`.

## Look…

The manuscript's format, also in *Esc › Settings › Manuscript look…*: **Modern** (Times) or **Classic** (Courier, underlined italics, straight quotes), US Letter or A4, double or 1.5 spacing, the chapter heading style (Chapter One, CHAPTER ONE, Chapter 1, 1, or the chapter's title), first paragraphs indented or flush, END / THE END / nothing, how a spaced hyphen is set, and the header keyword. `↑` `↓` choose, `←` `→` change, `Esc` saves.

## Author details…

Your byline (pen name), legal name, address, phone, email and an agent's details if you have one — for the title page. Saved in the book's `novel.toml`. Until they're filled in, the export dialog says so.

## PDF

A PDF is made with **LibreOffice** (free, from libreoffice.org). Without it, export makes the Word file and says how to get LibreOffice; Word can also save the file as a PDF.

From the command line: `grimoire export --docx --pdf --chapters 1-3` (or `--words 10000`, `--parts 1,3`, `--epub`, `--md`).
