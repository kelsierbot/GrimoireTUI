# Revision

Tools for going back over a draft.

## Next scene still in draft

*Next scene still in draft*, in *Esc › Writing tools* or `Ctrl-K`, opens the next scene in book order whose status isn't *revised* or *done* — empty scenes are skipped, and it goes round to the start when it reaches the end. Work through a revision pass by setting each scene's status as you finish it (`s` on the corkboard steps it, or edit `status:` in the frontmatter).

## Echo words

*Echo words* lights up a word you've used again within about forty words of the last time — the "the lamp… the lamp" that's easy to miss. Common little words and your characters' names are left out. Turn it on and off from *Esc › Writing tools* or `Ctrl-K`. It's off to begin with, and it only marks the page: nothing in the scene changes until you do.

## Read aloud

Hearing your prose catches what the eye skips: the missing word, the sentence that runs out of breath, the rhythm that's off. *Esc › Writing tools › Read aloud* (or `Ctrl-K` › *Read aloud from here*) reads the scene out loud from the sentence you're on, and highlights each sentence as it goes.

- `Space` pauses, and `Space` again goes on.
- `←` and `→` go back or forward a sentence.
- `Esc`, or any other key, stops. The cursor stays where it stopped, so you can fix what you heard.

Notes (`%% … %%`) are never read out, and neither are the `*` and `_` around italics.

### The voice

Grimoire reads with whatever voice your computer has, and nothing leaves your computer:

- **On a Mac** and **on Windows**, the computer's own voice, already there.
- **On Linux**, Speech Dispatcher or eSpeak if either is installed (`sudo apt install espeak-ng`, `sudo dnf install espeak-ng`).
- **Piper**, anywhere, for a voice that sounds like a person. It's free and works offline. Install it with `pipx install piper-tts`. Then make a folder called `piper-voices` in `~/.local/share`, go into it, and download a voice there: `python3 -m piper.download_voices en_US-lessac-medium`. Grimoire finds it by itself.

When there's more than one, Grimoire picks the most natural. *Esc › Settings › Reading voice…* chooses a different one.

## Other passes

- *Check names for near-miss spellings* finds names that have drifted (see *The notebook and names*).
- `Ctrl-T` lists every note and TK you left for yourself.
- `H` shows how a scene has changed.
- *Writing sessions* show what changed in each sitting.
