# Sprints & targets

## Word targets

- **The book**: `target_words` in `novel.toml` (80,000 to begin with). The status bar shows the book's words against it, with a progress bar.
- **Each day**: `daily_target` in `novel.toml`. *today* in the status bar is what the book has grown by since midnight — it goes below zero after a day of cutting, and turns to the accent colour once you reach the day's target.
- **A scene**: `target:` in its frontmatter. The page's title then shows the scene's progress, like `Scene One · 812 / 1,500`, and the corkboard shows it on the card.

Notes and TKs never count.

## Progress

*Esc › Writing tools › Progress…* (or a click on the word count at the bottom left, or `Ctrl-K` › *Progress*) shows how the book is coming along:

- The book's words against its goal, as a bar.
- **Each week**: what you wrote in each of the last eight weeks, as bars.
- **Your pace**: about how many words a week you've written lately, and roughly when you'll reach your goal at that pace.
- **Today**: what you've written so far, against what you aim for in a day.

It's there to encourage, never to nag. There are no streaks and no days in a row, and a quiet week is just a short bar. Grimoire starts keeping the record the first time you write with this version, one line a day in the book's hidden `.grimoire` folder, so the weeks fill in as you go.

## Sprints

*Start a sprint…* (menu or `Ctrl-K`) asks for a number of words and minutes — 500 words and the Pomodoro's length to begin with. `Tab` moves between the two, digits change them, `Enter` starts and `Esc` cancels.

The Pomodoro starts, and the status bar counts the sprint: `sprint 212/500 · 14:32`. Reaching the goal says so once, quietly. `F3` (or *Stop the sprint*) ends it early.

There are no streaks and nothing to keep up with — a sprint is just a sprint.
