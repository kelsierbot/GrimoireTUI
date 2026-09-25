# Focus mode & typewriter

## Focus mode

`Ctrl-D` hides everything but the prose: the outline, the Pomodoro and the music pane go away, and the scene sits centred on the screen with a quiet status line. `Ctrl-D` again (*Leave focus mode*) brings it all back. Both are in the menu and in `Ctrl-K` too.

`Esc` still opens the menu in focus mode, and closing the menu brings you straight back to the page.

## Line width

Long lines are hard to read, so the prose wraps at **72 columns** and sits centred in the page, however wide your window is. *Line width* in Settings cycles through 60, 72, 80, 100 and the whole pane.

## Typewriter scrolling

On a typewriter the line you're typing stays in one place and the paper moves up. With typewriter scrolling on, **in focus mode** the line you're writing stays in the middle of the screen: when you type past the middle, the text scrolls up instead of the cursor moving down. Your eyes stay at one height, and there's always room below the line you're on.

- It only applies in focus mode. In the normal layout Grimoire simply keeps a few blank rows below the cursor, so you're never writing on the bottom edge; the text scrolls only when you get near it.
- It moves the view only when the cursor moves, so scrolling back with the mouse to reread something doesn't snap you back.
- It's on to begin with. Turn it off in *Esc › Settings › Turn typewriter scrolling off*, or from `Ctrl-K`, and focus mode scrolls like the normal layout.

Both settings are remembered in `~/.config/grimoire/settings.toml` (`line_width` and `typewriter`).
