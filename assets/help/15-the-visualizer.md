# The Visualizer

The Pomodoro's other view (`→` in its pane) is a spectrum analyser for whatever your computer is playing, drawn in your theme's own style: meters, beads, frost, a neon skyline, a rolling wave, a synthwave grid…

It always **names the song** that's playing, in its title or written along the bottom, cut to fit.

## Where the sound comes from

It listens to the computer's own output, only while the view is on screen.

- **On a Mac** (macOS 14.6 or later): your terminal needs permission under *System Settings › Privacy & Security › Screen & System Audio Recording › System Audio Recording Only*. Until then the pane says it's hearing only silence.
- **On Linux**: through PipeWire's `pw-record`, or PulseAudio's `parec`, whichever is installed. To listen to an output other than the default, start Grimoire with `GRIMOIRE_MONITOR=<sink>` set.
- **On Windows**: not yet.

If it can't listen, the pane says why in one line, and `←` `→` still move on.

A custom theme can borrow any theme's Visualizer (see *Themes*).
