//! Reading the scene aloud: the sentence being read is highlighted on the
//! page, and a line along the bottom says what the keys do. And choosing the
//! voice.

use super::*;
use crate::voice::{self, Engine, Event};

impl App {
    /// Read the open scene aloud from the sentence the cursor is in.
    pub fn start_reading(&mut self) {
        if self.open.is_none() {
            self.msg = "open a scene to read it aloud".into();
            return;
        }
        let sentences = voice::sentences(&self.editor.lines);
        if sentences.is_empty() {
            self.msg = "there's nothing in this scene to read yet".into();
            return;
        }
        let here = (self.editor.cy, self.editor.cx);
        let from = sentences
            .iter()
            .position(|s| (s.line, s.end) > here)
            .unwrap_or(0);
        // Tests never make a sound.
        let engine = if self.background {
            match voice::choose(&Settings::load().voice) {
                Some(e) => e,
                None => {
                    self.msg = voice::how_to_get_one().into();
                    return;
                }
            }
        } else {
            Engine::Silent
        };
        let name = engine.name();
        let texts = sentences.iter().map(|s| s.text.clone()).collect();
        self.reading = Some(Reading {
            reader: voice::Reader::start(engine, texts, from),
            sentences,
            at: from,
            voice: name.clone(),
        });
        self.focus = Focus::Editor;
        self.show_sentence(from);
        self.overlay = Overlay::Reading;
        self.msg = format!("reading aloud · {name}");
    }

    fn show_sentence(&mut self, i: usize) {
        if let Some(s) = self.reading.as_ref().and_then(|r| r.sentences.get(i)) {
            let (line, start, end) = (s.line, s.start, s.end);
            self.editor.select((line, start), (line, end));
        }
    }

    /// What the voice has been up to since the last look.
    pub fn tick_reading(&mut self) {
        let Some(r) = &mut self.reading else {
            return;
        };
        for event in r.reader.events() {
            match event {
                Event::Reading(i) => {
                    if let Some(r) = &mut self.reading {
                        r.at = i;
                    }
                    self.show_sentence(i);
                }
                Event::Done => {
                    self.stop_reading("that's the end of the scene");
                    return;
                }
                Event::Failed(why) => {
                    self.stop_reading(&why);
                    return;
                }
            }
        }
        // Put away some other way (a book switched, say): stop.
        if !matches!(self.overlay, Overlay::Reading | Overlay::Help { .. }) {
            self.stop_reading("stopped reading");
        }
    }

    /// Stop, leaving the cursor at the end of the sentence it was on.
    pub fn stop_reading(&mut self, note: &str) {
        if self.reading.take().is_some() {
            self.editor.clear_selection();
            if self.overlay == Overlay::Reading {
                self.overlay = Overlay::None;
            }
            self.msg = note.to_string();
        }
    }

    pub(super) fn on_reading_key(&mut self, key: Key) {
        let Some(r) = &mut self.reading else {
            self.overlay = Overlay::None;
            return;
        };
        match key {
            Key::Char(' ') => {
                r.reader.pause_or_resume();
                self.msg = if r.reader.paused {
                    "paused · space goes on".into()
                } else {
                    format!("reading aloud · {}", r.voice)
                };
            }
            Key::Left | Key::Up => {
                let i = r.at.saturating_sub(1);
                r.reader.jump(i);
            }
            Key::Right | Key::Down => {
                let i = r.at + 1;
                r.reader.jump(i);
            }
            _ => self.stop_reading("stopped reading"),
        }
    }

    pub fn open_voices(&mut self) {
        let list = if self.background {
            voice::available()
        } else {
            vec![Engine::Silent]
        };
        let chosen = if self.background {
            Settings::load().voice
        } else {
            String::new()
        };
        // The first row is "the most natural one here"; then each voice.
        let sel = list
            .iter()
            .position(|e| !chosen.is_empty() && e.key() == chosen)
            .map_or(0, |i| i + 1);
        self.overlay = Overlay::Voices { sel, list };
    }

    pub(super) fn on_voices_key(&mut self, key: Key) {
        let Overlay::Voices { sel, list } = &mut self.overlay else {
            return;
        };
        if list_nav(key, sel, list.len() + 1, RING) {
            return;
        }
        match key {
            Key::Enter | Key::Char(' ') => {
                let (key, name) = match list.get(sel.wrapping_sub(1)) {
                    Some(e) if *sel > 0 => (e.key(), e.name()),
                    _ => (String::new(), "the most natural voice here".to_string()),
                };
                if self.background {
                    Self::save_voice(key);
                }
                self.overlay = Overlay::None;
                self.msg = format!("Read aloud will use {name}");
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }

    fn save_voice(key: String) {
        let mut s = Settings::load();
        s.voice = key;
        let _ = s.save();
    }
}

/// Along the bottom, over the status bar: which sentence, and the keys.
pub(super) fn draw_reading(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Some(r) = &app.reading else {
        return;
    };
    let row = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(1),
        area.width,
        1,
    );
    f.render_widget(Clear, row);
    let mut spans = vec![
        Span::styled(" ♪ ", Style::default().fg(t.accent)),
        Span::styled(
            if r.reader.paused {
                "paused"
            } else {
                "reading aloud"
            },
            Style::default().fg(t.accent),
        ),
        Span::styled(
            format!(
                " · sentence {} of {} · {}   ",
                r.at + 1,
                r.sentences.len(),
                r.voice
            ),
            Style::default().fg(t.dim),
        ),
    ];
    spans.extend(hint_spans(
        if r.reader.paused {
            "space go on   ← → sentence   esc stop"
        } else {
            "space pause   ← → sentence   esc stop"
        },
        t,
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), row);
}

pub(super) fn draw_voices(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Voices { sel, list } = &app.overlay else {
        return;
    };
    let mut rows: Vec<String> = vec!["Automatic: the most natural one here".into()];
    rows.extend(list.iter().map(Engine::name));
    let empty = list.is_empty();
    let box_area = centred(area, 60, rows.len() as u16 + 6 + if empty { 2 } else { 0 });
    f.render_widget(Clear, box_area);
    let block = pane_block("READING VOICE", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            " Who reads your scenes aloud",
            Style::default().fg(t.dim),
        )),
        Line::from(""),
    ];
    for (i, name) in rows.iter().enumerate() {
        let on = i == *sel;
        lines.push(
            Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
                Span::styled(
                    name.clone(),
                    Style::default().fg(if on { t.accent } else { t.text }),
                ),
            ])
            .style(if on {
                Style::default().bg(t.sel)
            } else {
                Style::default()
            }),
        );
    }
    if empty {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " No voices found yet. Help › Revision says how to get one.",
            Style::default().fg(t.warn),
        )));
    }
    lines.push(Line::from(""));
    lines.push(hint_line(" j/k move   ↵ choose   esc close", t));
    f.render_widget(Paragraph::new(lines), inner);
}
