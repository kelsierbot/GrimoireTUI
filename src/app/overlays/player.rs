//! The music player: now playing, the queue, playlists and search.

use super::*;

impl App {
    pub(super) fn on_player_key(&mut self, key: Key) {
        let Overlay::Player {
            tab,
            sel,
            follow,
            query,
            find,
            typing,
        } = &mut self.overlay
        else {
            return;
        };
        use music::Cmd;
        let len = match tab {
            Tab::Queue => self.music.queue.len(),
            Tab::Playlists => self.music.playlists.len(),
            Tab::Search => self.music.results.len(),
        };
        if *typing {
            // The playlist tab types into its finder; the others, into search.
            let on_playlists = *tab == Tab::Playlists;
            let buf = if on_playlists { find } else { query };
            match key {
                Key::Char(c) if !c.is_control() && buf.chars().count() < 80 => buf.push(c),
                Key::Backspace => {
                    buf.pop();
                }
                Key::Enter => {
                    *typing = false;
                    *sel = 0;
                    let q = buf.trim().to_string();
                    if on_playlists {
                        self.music.playlists.clear();
                        self.music.note = Some(if q.is_empty() {
                            "fetching your playlists…".into()
                        } else {
                            format!("searching playlists for “{q}”…")
                        });
                        self.music
                            .send(Cmd::Playlists((!q.is_empty()).then_some(q)));
                    } else if !q.is_empty() {
                        self.music.results.clear();
                        self.music.note = Some(format!("searching for “{q}”…"));
                        self.music.send(Cmd::Search(q));
                    }
                }
                Key::Esc => *typing = false,
                _ => {}
            }
            return;
        }
        match key {
            // From a playlist search, Esc goes back to your own first.
            Key::Esc if *tab == Tab::Playlists && !find.is_empty() => {
                find.clear();
                *sel = 0;
                self.music.note = Some("fetching your playlists…".into());
                self.music.send(Cmd::Playlists(None));
            }
            Key::Esc | Key::F(7) => self.overlay = Overlay::None,
            Key::Tab | Key::BackTab => {
                *tab = if key == Key::Tab {
                    tab.next()
                } else {
                    tab.prev()
                };
                *sel = 0;
                *follow = *tab == Tab::Queue;
                if *tab == Tab::Playlists && self.music.playlists.is_empty() {
                    self.music.note = Some("fetching your playlists…".into());
                    self.music.send(Cmd::Playlists(None));
                }
            }
            Key::Char('/') => {
                if *tab != Tab::Playlists {
                    *tab = Tab::Search;
                    *sel = 0;
                }
                *follow = false;
                *typing = true;
            }
            // Moving the selection yourself stops it following the song.
            k if list_nav(k, sel, len, LIST) => *follow = false,
            Key::Enter => match *tab {
                Tab::Queue => {
                    if let Some(it) = self.music.queue.get(*sel) {
                        *follow = true;
                        self.music.send(Cmd::JumpTo(it.pos));
                    }
                }
                Tab::Search => {
                    if let Some(it) = self.music.results.get(*sel) {
                        self.music.note = Some(format!("playing {}", it.title));
                        self.music.send(Cmd::Enqueue {
                            id: it.id.clone(),
                            now: true,
                        });
                    }
                }
                Tab::Playlists => {
                    if let Some(it) = self.music.playlists.get(*sel) {
                        self.music.note = Some(format!("loading {}…", it.title));
                        self.music.send(Cmd::Playlist {
                            id: it.id.clone(),
                            title: it.title.clone(),
                            now: true,
                        });
                        // Over to the queue, to watch it fill.
                        *tab = Tab::Queue;
                        *sel = 0;
                        *follow = true;
                    }
                }
            },
            Key::Char('a') if *tab == Tab::Search => {
                if let Some(it) = self.music.results.get(*sel) {
                    self.music.note = Some(format!("up next: {}", it.title));
                    self.music.send(Cmd::Enqueue {
                        id: it.id.clone(),
                        now: false,
                    });
                }
            }
            Key::Char('a') if *tab == Tab::Playlists => {
                if let Some(it) = self.music.playlists.get(*sel) {
                    self.music.note = Some(format!("queueing {} after this song…", it.title));
                    self.music.send(Cmd::Playlist {
                        id: it.id.clone(),
                        title: it.title.clone(),
                        now: false,
                    });
                }
            }
            Key::Char(' ') => self.music.send(Cmd::PlayPause),
            Key::Right => self.music.send(Cmd::Seek(10)),
            Key::Left => self.music.send(Cmd::Seek(-10)),
            Key::Char(']') | Key::Char('n') => self.music.send(Cmd::Next),
            Key::Char('[') | Key::Char('p') => self.music.send(Cmd::Prev),
            Key::Char('s') => self.music.send(Cmd::Shuffle),
            Key::Char('r') => self.music.send(Cmd::Repeat),
            Key::Char('+') | Key::Char('=') => self.music.send(Cmd::Volume(10)),
            Key::Char('-') => self.music.send(Cmd::Volume(-10)),
            Key::Char('l') => {
                self.music.note = Some("liked".into());
                self.music.send(Cmd::Like);
            }
            Key::F(n) => self.on_function_key(n),
            _ => {}
        }
    }
}

pub(super) fn draw_player(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Player {
        tab,
        sel,
        query,
        find,
        typing,
        ..
    } = &app.overlay
    else {
        return;
    };
    let box_area = centred(
        area,
        area.width.saturating_sub(4).min(100),
        area.height.saturating_sub(2).min(30),
    );
    f.render_widget(Clear, box_area);
    let title = format!("♪ {}", app.music.source.label().to_uppercase());
    let block = pane_block(&title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let iw = inner.width as usize;
    let dim = Style::default().fg(t.dim);
    let accent = Style::default().fg(t.accent);
    let mins = |s: f64| format!("{:.0}:{:02.0}", (s / 60.0).floor(), s % 60.0);
    let mut lines: Vec<Line> = Vec::new();

    // Now playing, with a real progress bar.
    match &app.music.state {
        MusicState::Playing(tr) => {
            lines.push(Line::from(vec![
                Span::styled(if tr.playing { " ▶ " } else { " ❚❚ " }, accent),
                Span::styled(
                    truncate(&tr.title, iw.saturating_sub(26).max(8)),
                    Style::default().fg(t.text),
                ),
                Span::styled(format!("  {}", truncate(&tr.artist, 22)), dim),
            ]));
            let times = format!(" {} / {}", mins(tr.progress), mins(tr.duration));
            let barw = iw.saturating_sub(times.chars().count() + 1).max(4);
            let frac = if tr.duration > 0.0 {
                (tr.progress / tr.duration).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let filled = ((frac * barw as f64).round() as usize).min(barw);
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled("━".repeat(filled), accent),
                Span::styled("─".repeat(barw - filled), Style::default().fg(t.border)),
                Span::styled(times, dim),
            ]));
        }
        other => {
            let why = match other {
                MusicState::NoToken => {
                    "not set up yet: Esc › Settings › Music source, then run grimoire music-setup"
                        .to_string()
                }
                MusicState::Offline => format!(
                    "{} isn't running. Open it and this fills in.",
                    app.music.source.label()
                ),
                _ => "nothing playing. Pick something below.".to_string(),
            };
            lines.push(Line::from(Span::styled(format!(" {why}"), dim)));
            lines.push(Line::from(""));
        }
    }
    lines.push(Line::from(""));

    // Tabs, and what the list below is showing.
    use crate::app::Tab;
    let on =
        accent.add_modifier(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED);
    let style = |which: Tab| if *tab == which { on } else { dim };
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("QUEUE · {}", app.music.queue.len()),
            style(Tab::Queue),
        ),
        Span::raw("    "),
        Span::styled("PLAYLISTS", style(Tab::Playlists)),
        Span::raw("    "),
        Span::styled("SEARCH", style(Tab::Search)),
        Span::raw("    "),
        Span::styled("tab", key_style(t)),
        Span::styled(" switches", dim),
    ]));
    let prompt = |label: &str, buf: &str| {
        Line::from(vec![
            Span::styled(format!(" {label} ▸ "), accent),
            Span::styled(buf.to_string(), Style::default().fg(t.text)),
            Span::styled("█", accent),
        ])
    };
    match tab {
        Tab::Search => lines.push(if *typing {
            prompt("find", query)
        } else if query.is_empty() {
            Line::from(Span::styled(" press / and type a song or an artist", dim))
        } else {
            Line::from(Span::styled(
                format!(" results for “{query}” · / to search again"),
                dim,
            ))
        }),
        Tab::Playlists => lines.push(if *typing {
            prompt("playlists", find)
        } else if find.is_empty() {
            Line::from(Span::styled(
                " your library · / searches every playlist on YouTube Music",
                dim,
            ))
        } else {
            Line::from(Span::styled(
                format!(" playlists matching “{find}” · esc for yours"),
                dim,
            ))
        }),
        Tab::Queue => {}
    }
    lines.push(Line::from(""));

    // The list, scrolled to keep the selection in view.
    let footer = 3;
    let room = (inner.height as usize)
        .saturating_sub(lines.len() + footer)
        .max(1);
    let items = match tab {
        Tab::Queue => &app.music.queue,
        Tab::Playlists => &app.music.playlists,
        Tab::Search => &app.music.results,
    };
    if items.is_empty() && *tab == Tab::Queue {
        lines.push(Line::from(Span::styled(
            " the queue is empty. Tab over to your playlists, or search with /",
            dim,
        )));
    }
    let start = sel
        .saturating_sub(room / 2)
        .min(items.len().saturating_sub(room));
    // Playlists show a song count where tracks show a length.
    let len_w = if *tab == Tab::Playlists { 11 } else { 6 };
    let artist_w = (iw / 4).clamp(8, 28);
    let title_w = iw.saturating_sub(artist_w + 8 + len_w);
    for (i, it) in items.iter().enumerate().skip(start).take(room) {
        let who = if it.video {
            format!("{} · video", it.artist)
        } else {
            it.artist.clone()
        };
        let row = Line::from(vec![
            Span::styled(
                format!(" {}{:>3} ", if it.current { "▶" } else { " " }, i + 1),
                if it.current { accent } else { dim },
            ),
            Span::styled(
                format!("{:<title_w$}", truncate(&it.title, title_w)),
                Style::default().fg(if it.current { t.accent } else { t.text }),
            ),
            Span::styled(format!(" {:<artist_w$}", truncate(&who, artist_w)), dim),
            Span::styled(format!("{:>len_w$}", it.length), dim),
        ]);
        lines.push(if i == *sel {
            row.style(Style::default().bg(t.sel))
        } else {
            row
        });
    }

    while lines.len() < (inner.height as usize).saturating_sub(footer) {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        format!(" {}", app.music.note.clone().unwrap_or_default()),
        accent,
    )));
    let controls = " space pause  ←→ seek 10s  [ ] prev/next  s shuffle  r repeat  +/- volume  l like  esc close";
    let keys = match tab {
        Tab::Queue => " ↵ play this one   ↑↓ choose   / search   tab playlists",
        Tab::Playlists => {
            " ↵ play playlist   a play it next   / find playlists   ↑↓ choose   tab search"
        }
        Tab::Search => " / search   ↵ play now   a play next   ↑↓ choose   tab queue",
    };
    lines.extend([keys, controls].map(|k| hint_line(k, t)));
    f.render_widget(Paragraph::new(lines), inner);
}
