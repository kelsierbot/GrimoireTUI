//! Reading a scene aloud, a sentence at a time, with the sentence being read
//! highlighted on the page. Hearing prose catches what the eye skips.
//!
//! The voice is whatever the computer has: Piper (natural, and offline) when
//! it's installed with a voice, otherwise the system's own — `say` on a Mac,
//! Speech Dispatcher or eSpeak on Linux, the Windows voice. Grimoire only
//! hands it text; nothing leaves the computer.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// One sentence of the scene: where it sits in the editor (a line and a
/// char range in it), and the words that are said.
#[derive(Debug, Clone, PartialEq)]
pub struct Sentence {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Words that end in a full stop without ending the sentence.
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "st", "mt", "jr", "sr", "prof", "vs", "etc", "e.g", "i.e", "no",
];

/// The scene's sentences, in order. Notes (`%% … %%`) and Markdown's marks
/// aren't said, and a sentence with nothing left to say is skipped.
pub fn sentences(lines: &[String]) -> Vec<Sentence> {
    let mut out = Vec::new();
    for (line, text) in lines.iter().enumerate() {
        let chars: Vec<char> = text.chars().collect();
        let mut start = 0;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let mut end = None;
            if matches!(c, '.' | '!' | '?' | '…') {
                let mut j = i + 1;
                while j < chars.len() && matches!(chars[j], '.' | '!' | '?' | '…') {
                    j += 1;
                }
                while j < chars.len()
                    && matches!(chars[j], '"' | '\'' | '”' | '’' | ')' | ']' | '*' | '_')
                {
                    j += 1;
                }
                if j == chars.len() || chars[j].is_whitespace() {
                    let word: String = chars[start..i]
                        .iter()
                        .rev()
                        .take_while(|c| !c.is_whitespace())
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                        .trim_start_matches(['"', '“', '(', '\''])
                        .to_lowercase();
                    if !(c == '.' && ABBREVIATIONS.contains(&word.as_str())) {
                        end = Some(j);
                    }
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
            if let Some(e) = end.or((i >= chars.len()).then_some(chars.len())) {
                push(&mut out, line, &chars, start, e);
                start = e;
                while start < chars.len() && chars[start].is_whitespace() {
                    start += 1;
                }
                i = i.max(start);
            }
        }
    }
    out
}

fn push(out: &mut Vec<Sentence>, line: usize, chars: &[char], start: usize, end: usize) {
    let raw: String = chars[start..end].iter().collect();
    let said = grimoire_core::notes::strip_notes(&raw)
        .replace(['*', '_', '#'], "")
        .replace("%%", "");
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    if said.chars().any(char::is_alphanumeric) {
        let lead = raw.chars().take_while(|c| c.is_whitespace()).count();
        out.push(Sentence {
            line,
            start: start + lead,
            end,
            text: said,
        });
    }
}

/// A voice to read with.
#[derive(Debug, Clone, PartialEq)]
pub enum Engine {
    /// Piper, with one of its voice models.
    Piper { program: PathBuf, model: PathBuf },
    /// macOS's own voice.
    Say,
    /// Linux's Speech Dispatcher (`spd-say`), with whatever voice it's set to.
    SpeechDispatcher,
    /// eSpeak NG.
    Espeak,
    /// The Windows voice, through PowerShell.
    Windows,
    /// Says nothing, and moves on only when told: for tests, which must
    /// never make a sound.
    Silent,
}

impl Engine {
    /// How the choice is kept in settings.toml.
    pub fn key(&self) -> String {
        match self {
            Engine::Piper { model, .. } => model.display().to_string(),
            Engine::Say => "say".into(),
            Engine::SpeechDispatcher => "spd-say".into(),
            Engine::Espeak => "espeak-ng".into(),
            Engine::Windows => "windows".into(),
            Engine::Silent => "silent".into(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Engine::Piper { model, .. } => {
                // "en_US-lessac-high" is "Lessac (Piper, high quality)".
                let stem = model
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let mut bits = stem.split('-');
                let _lang = bits.next();
                let who = bits.next().unwrap_or(&stem);
                let quality = bits.next().unwrap_or("");
                let who = who
                    .split('_')
                    .map(|w| {
                        let mut c = w.chars();
                        c.next()
                            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                if quality.is_empty() {
                    format!("{who} (Piper)")
                } else {
                    format!("{who} (Piper, {quality} quality)")
                }
            }
            Engine::Say => "The Mac's voice".into(),
            Engine::SpeechDispatcher => "Speech Dispatcher".into(),
            Engine::Espeak => "eSpeak".into(),
            Engine::Windows => "The Windows voice".into(),
            Engine::Silent => "Silent".into(),
        }
    }
}

/// A program on the PATH, or in the usual place `pipx` and `pip --user` put
/// them.
fn program(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([grimoire_core::paths::home().join(".local").join("bin")])
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// Piper voice models: an `.onnx` with its `.onnx.json` beside it, in the
/// places voices are usually downloaded to.
fn piper_voices() -> Vec<PathBuf> {
    let home = grimoire_core::paths::home();
    let roots = [
        grimoire_core::paths::data_dir().join("voices"),
        home.join(".local/share/piper-voices"),
        home.join(".local/share/piper"),
        home.join("piper-voices"),
        home.join("Models"),
        home.join("models"),
    ];
    let mut out = Vec::new();
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth > 0 {
                walk(&p, depth - 1, out);
            } else if p.extension().is_some_and(|x| x == "onnx") {
                let config = p.with_extension("onnx.json");
                // A Piper voice's config names its phonemes; other models don't.
                if std::fs::read_to_string(&config).is_ok_and(|c| c.contains("\"espeak\"")) {
                    out.push(p);
                }
            }
        }
    }
    for r in roots {
        walk(&r, 3, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// Every voice this computer has, the most natural first.
pub fn available() -> Vec<Engine> {
    let mut out = Vec::new();
    if let Some(piper) = program("piper") {
        for model in piper_voices() {
            out.push(Engine::Piper {
                program: piper.clone(),
                model,
            });
        }
    }
    if cfg!(target_os = "macos") {
        out.push(Engine::Say);
    }
    if cfg!(windows) {
        out.push(Engine::Windows);
    }
    if cfg!(all(unix, not(target_os = "macos"))) {
        if program("spd-say").is_some() {
            out.push(Engine::SpeechDispatcher);
        }
        if program("espeak-ng").is_some() {
            out.push(Engine::Espeak);
        }
    }
    out
}

/// The voice to read with: the one chosen in Settings if it's still here,
/// otherwise the most natural one available.
pub fn choose(setting: &str) -> Option<Engine> {
    let all = available();
    all.iter()
        .find(|e| !setting.is_empty() && e.key() == setting)
        .or(all.first())
        .cloned()
}

/// What to do when there's no voice at all.
pub fn how_to_get_one() -> &'static str {
    if cfg!(target_os = "macos") || cfg!(windows) {
        "no voice to read with was found"
    } else {
        "no voice to read with yet · install espeak-ng, or Piper for a natural one (Help › Revision)"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Sentence `i` has started.
    Reading(usize),
    /// The last sentence is done.
    Done,
    Failed(String),
}

enum Cmd {
    Pause,
    Resume,
    Jump(usize),
    Stop,
}

/// A reading in progress, on a thread of its own. Dropping it stops it.
pub struct Reader {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
    pub paused: bool,
}

impl Reader {
    pub fn start(engine: Engine, texts: Vec<String>, from: usize) -> Reader {
        let (tx, cmds) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = match engine {
                Engine::Silent => silent(&texts, from, &cmds, &events),
                Engine::Piper { program, model } => {
                    piper(&program, &model, &texts, from, &cmds, &events)
                }
                Engine::Windows => windows(&texts, from, &cmds, &events),
                other => one_at_a_time(&other, &texts, from, &cmds, &events),
            };
            if let Err(e) = result {
                let _ = events.send(Event::Failed(e));
            }
        });
        Reader {
            tx,
            rx,
            paused: false,
        }
    }

    pub fn pause_or_resume(&mut self) {
        self.paused = !self.paused;
        let _ = self
            .tx
            .send(if self.paused { Cmd::Pause } else { Cmd::Resume });
    }

    pub fn jump(&mut self, i: usize) {
        let _ = self.tx.send(Cmd::Jump(i));
    }

    pub fn events(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
    }
}

const POLL: Duration = Duration::from_millis(25);

fn silent(
    texts: &[String],
    from: usize,
    cmds: &Receiver<Cmd>,
    events: &Sender<Event>,
) -> Result<(), String> {
    let _ = events.send(Event::Reading(from));
    while let Ok(cmd) = cmds.recv() {
        match cmd {
            Cmd::Jump(j) if j < texts.len() => {
                let _ = events.send(Event::Reading(j));
            }
            Cmd::Jump(_) => {
                let _ = events.send(Event::Done);
                return Ok(());
            }
            Cmd::Stop => return Ok(()),
            Cmd::Pause | Cmd::Resume => {}
        }
    }
    Ok(())
}

/// Voices that say one thing and exit: a process per sentence, the text on
/// its standard input.
fn one_at_a_time(
    engine: &Engine,
    texts: &[String],
    from: usize,
    cmds: &Receiver<Cmd>,
    events: &Sender<Event>,
) -> Result<(), String> {
    let spawn = |text: &str| -> Result<Child, String> {
        let mut c = match engine {
            Engine::Say => Command::new("say"),
            Engine::SpeechDispatcher => {
                let mut c = Command::new("spd-say");
                c.args(["-w", "-e"]);
                c
            }
            _ => {
                let mut c = Command::new("espeak-ng");
                c.arg("--stdin");
                c
            }
        };
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't start the voice ({e})"))?;
        if let Some(mut input) = child.stdin.take() {
            let _ = writeln!(input, "{text}");
        }
        Ok(child)
    };
    // Speech Dispatcher keeps talking after its client is gone.
    let hush = |child: &mut Option<Child>| {
        if let Some(mut c) = child.take() {
            let _ = c.kill();
            let _ = c.wait();
            if *engine == Engine::SpeechDispatcher {
                let _ = Command::new("spd-say").arg("-C").status();
            }
        }
    };
    let (mut i, mut paused, mut child): (usize, bool, Option<Child>) = (from, false, None);
    loop {
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                // Paused mid-sentence, it starts that sentence again.
                Cmd::Pause => {
                    hush(&mut child);
                    paused = true;
                }
                Cmd::Resume => paused = false,
                Cmd::Jump(j) => {
                    hush(&mut child);
                    i = j;
                }
                Cmd::Stop => {
                    hush(&mut child);
                    return Ok(());
                }
            }
        }
        if !paused && child.is_none() {
            if i >= texts.len() {
                let _ = events.send(Event::Done);
                return Ok(());
            }
            let _ = events.send(Event::Reading(i));
            child = Some(spawn(&texts[i])?);
        }
        if let Some(c) = &mut child
            && c.try_wait().ok().flatten().is_some()
        {
            child = None;
            i += 1;
        }
        std::thread::sleep(POLL);
    }
}

/// The Windows voice, kept open in one PowerShell: it says each line it's
/// given and answers `done`.
fn windows(
    texts: &[String],
    from: usize,
    cmds: &Receiver<Cmd>,
    events: &Sender<Event>,
) -> Result<(), String> {
    const SCRIPT: &str = "Add-Type -AssemblyName System.Speech; \
        $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
        while ($null -ne ($l = [Console]::In.ReadLine())) { $s.Speak($l); [Console]::Out.WriteLine('done'); [Console]::Out.Flush() }";
    struct Voice {
        child: Child,
        input: ChildStdin,
        done: Receiver<()>,
    }
    let open = || -> Result<Voice, String> {
        let mut child = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't start the Windows voice ({e})"))?;
        let input = child.stdin.take().ok_or("no input to the voice")?;
        let output = child.stdout.take().ok_or("no answer from the voice")?;
        let (tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                if line.is_err() || tx.send(()).is_err() {
                    break;
                }
            }
        });
        Ok(Voice { child, input, done })
    };
    let close = |v: &mut Option<Voice>| {
        if let Some(mut v) = v.take() {
            let _ = v.child.kill();
            let _ = v.child.wait();
        }
    };
    let (mut i, mut paused, mut speaking) = (from, false, false);
    let mut voice: Option<Voice> = None;
    loop {
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                Cmd::Pause => {
                    close(&mut voice);
                    speaking = false;
                    paused = true;
                }
                Cmd::Resume => paused = false,
                Cmd::Jump(j) => {
                    close(&mut voice);
                    speaking = false;
                    i = j;
                }
                Cmd::Stop => {
                    close(&mut voice);
                    return Ok(());
                }
            }
        }
        if speaking && voice.as_ref().is_some_and(|v| v.done.try_recv().is_ok()) {
            speaking = false;
            i += 1;
        }
        if !paused && !speaking {
            if i >= texts.len() {
                close(&mut voice);
                let _ = events.send(Event::Done);
                return Ok(());
            }
            if voice.is_none() {
                voice = Some(open()?);
            }
            let v = voice.as_mut().expect("just opened");
            // One line each: the voice reads to the end of the line.
            writeln!(v.input, "{}", texts[i].replace(['\r', '\n'], " "))
                .and_then(|()| v.input.flush())
                .map_err(|e| format!("the voice stopped ({e})"))?;
            let _ = events.send(Event::Reading(i));
            speaking = true;
        }
        std::thread::sleep(POLL);
    }
}

/// Piper, kept running with the voice loaded, writes each line it's given
/// to a WAV file of its own in a folder; Grimoire plays them in order. After
/// each sentence it's also given a word to say that's never played: its file
/// appearing means the sentence's is finished.
#[cfg(feature = "audio")]
fn piper(
    program: &Path,
    model: &Path,
    texts: &[String],
    from: usize,
    cmds: &Receiver<Cmd>,
    events: &Sender<Event>,
) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!(
        "grimoire-voice-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't make room for the voice ({e})"))?;
    let mut child = Command::new(program)
        .arg("-m")
        .arg(model)
        .arg("-d")
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start Piper ({e})"))?;
    let mut input = child.stdin.take().ok_or("no input to Piper")?;
    let device = rodio::DeviceSinkBuilder::open_default_sink()
        .map_err(|e| format!("no audio output available ({e})"))?;
    let player = rodio::Player::connect_new(device.mixer());

    let files = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<(u128, PathBuf)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let n = p.file_stem()?.to_str()?.parse::<u128>().ok()?;
                (p.extension()? == "wav").then_some((n, p))
            })
            .collect();
        v.sort();
        v.into_iter().map(|(_, p)| p).collect()
    };

    let result = (|| {
        let mut sent: Vec<usize> = Vec::new();
        let (mut i, mut paused) = (from, false);
        let mut playing: Option<usize> = None;
        loop {
            while let Ok(cmd) = cmds.try_recv() {
                match cmd {
                    Cmd::Pause => {
                        player.pause();
                        paused = true;
                    }
                    Cmd::Resume => {
                        player.play();
                        paused = false;
                    }
                    Cmd::Jump(j) => {
                        player.clear();
                        playing = None;
                        i = j;
                    }
                    Cmd::Stop => return Ok(()),
                }
            }
            // Keep the voice a couple of sentences ahead of what's heard.
            for (k, text) in texts.iter().enumerate().skip(i).take(3) {
                if !sent.contains(&k) {
                    writeln!(input, "{}\na", text.replace(['\r', '\n'], " "))
                        .and_then(|()| input.flush())
                        .map_err(|e| format!("Piper stopped ({e})"))?;
                    sent.push(k);
                }
            }
            if child.try_wait().ok().flatten().is_some() {
                return Err("Piper stopped: is the voice file whole?".to_string());
            }
            if playing.is_some() && player.empty() {
                playing = None;
                i += 1;
            }
            if playing.is_none() && i >= texts.len() {
                let _ = events.send(Event::Done);
                return Ok(());
            }
            if playing.is_none() && !paused {
                let pair = sent.iter().position(|&k| k == i).unwrap_or(usize::MAX);
                let made = files(&dir);
                if let (Some(wav), Some(_after)) = (made.get(2 * pair), made.get(2 * pair + 1)) {
                    let file = std::fs::File::open(wav).map_err(|e| e.to_string())?;
                    let sound = rodio::Decoder::new(std::io::BufReader::new(file))
                        .map_err(|e| format!("couldn't play the voice ({e})"))?;
                    player.append(sound);
                    player.play();
                    playing = Some(i);
                    let _ = events.send(Event::Reading(i));
                }
            }
            std::thread::sleep(POLL);
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(not(feature = "audio"))]
fn piper(
    _program: &Path,
    _model: &Path,
    _texts: &[String],
    _from: usize,
    _cmds: &Receiver<Cmd>,
    _events: &Sender<Event>,
) -> Result<(), String> {
    Err("this build has no audio, so Piper can't be heard".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(lines: &[&str]) -> Vec<String> {
        let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        sentences(&lines).into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn prose_splits_into_sentences_quotes_and_all() {
        assert_eq!(
            said(&[
                "The rain had stopped. \"You're early,\" said a voice. Mr. Oren waited!",
                "",
                "Was it *really* nine? %% check the time %% It was…",
            ]),
            [
                "The rain had stopped.",
                "\"You're early,\" said a voice.",
                "Mr. Oren waited!",
                "Was it really nine?",
                "It was…",
            ]
        );
    }

    #[test]
    fn a_sentence_knows_where_it_is_on_the_page() {
        let lines = vec!["One. Two three.".to_string()];
        let s = sentences(&lines);
        assert_eq!((s[1].line, s[1].start, s[1].end), (0, 5, 15));
        assert_eq!(&lines[0][5..15], "Two three.");
    }

    #[test]
    fn a_note_on_its_own_is_never_said() {
        assert_eq!(
            said(&["%% fix this later %%", "Then she left."]),
            ["Then she left."]
        );
    }

    #[test]
    fn voices_are_named_for_people() {
        let e = Engine::Piper {
            program: "piper".into(),
            model: "/v/en_US-lessac-high.onnx".into(),
        };
        assert_eq!(e.name(), "Lessac (Piper, high quality)");
        assert_eq!(e.key(), "/v/en_US-lessac-high.onnx");
    }
}

/// Piper end to end, with sound going nowhere:
///
///     ALSA_CONFIG_PATH=<a config whose default PCM is `type null`> \
///     GRIMOIRE_VOICE_TEST=/path/to/en_US-voice.onnx \
///     cargo test --bin grimoire piper_reads -- --ignored
#[cfg(all(test, feature = "audio"))]
#[test]
#[ignore]
fn piper_reads_every_sentence_in_order() {
    let Ok(model) = std::env::var("GRIMOIRE_VOICE_TEST") else {
        return;
    };
    assert!(
        std::env::var("ALSA_CONFIG_PATH").is_ok(),
        "only with the null ALSA config: this must never reach real speakers"
    );
    let program = program("piper").expect("piper on the PATH");
    let texts: Vec<String> = ["The rain had stopped.", "Oren was late.", "She waited."]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let r = Reader::start(
        Engine::Piper {
            program,
            model: model.into(),
        },
        texts,
        0,
    );
    let mut seen = Vec::new();
    let give_up = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < give_up {
        for e in r.events() {
            seen.push(e.clone());
            if matches!(e, Event::Done | Event::Failed(_)) {
                assert_eq!(
                    seen,
                    [
                        Event::Reading(0),
                        Event::Reading(1),
                        Event::Reading(2),
                        Event::Done
                    ]
                );
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("no end: {seen:?}");
}
