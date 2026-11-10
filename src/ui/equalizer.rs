//! Keyboard controls and honest live state for the native stereo equalizer.
use super::*;
use rustune_core::equalizer::{EqSettings, FREQUENCIES, PRESETS};
impl App {
    fn eq_settings(&self) -> EqSettings {
        self.eq_pending
            .as_ref()
            .unwrap_or(&self.config.equalizer)
            .clone()
    }
    pub(super) fn equalizer_key(&mut self, code: KeyCode) -> Option<Request> {
        let mut settings = self.eq_settings();
        if let Some(index) = (0..PRESETS.len()).find(|i| {
            EqSettings::preset(*i).bands_db == settings.bands_db && settings.preamp_db == 0.0
        }) {
            self.eq_preset = index;
        }
        let direction = match code {
            KeyCode::Left => -1.0,
            KeyCode::Right => 1.0,
            _ => 0.0,
        };
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.eq_selected = self.eq_selected.saturating_sub(1);
                return None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.eq_selected = (self.eq_selected + 1).min(12);
                return None;
            }
            KeyCode::Char('b') => settings.enabled = !settings.enabled,
            KeyCode::Home | KeyCode::Char('r') => {
                self.eq_preset = 0;
                settings = EqSettings::preset(0);
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Enter => match self.eq_selected {
                0 => settings.enabled = !settings.enabled,
                1 => {
                    self.eq_preset = if direction < 0.0 {
                        (self.eq_preset + PRESETS.len() - 1) % PRESETS.len()
                    } else {
                        (self.eq_preset + 1) % PRESETS.len()
                    };
                    settings = EqSettings::preset(self.eq_preset);
                }
                2..=11 => {
                    let band = self.eq_selected - 2;
                    settings.bands_db[band] = if code == KeyCode::Enter {
                        0.0
                    } else {
                        (settings.bands_db[band] + direction * 0.5).clamp(-12.0, 12.0)
                    };
                    settings.enabled = true;
                }
                _ => {
                    settings.preamp_db = if code == KeyCode::Enter {
                        0.0
                    } else {
                        (settings.preamp_db + direction * 0.5).clamp(-12.0, 12.0)
                    };
                    settings.enabled = true;
                }
            },
            _ => return None,
        }
        self.config.equalizer = settings.clone();
        self.eq_pending = Some(settings.clone());
        Some(Request::Equalizer(settings))
    }
    pub(super) fn render_equalizer(&self, f: &mut Frame, area: Rect, p: Palette) {
        let settings = self.eq_settings();
        let status = self.status.as_ref();
        let live = status.is_some_and(|s| {
            s.audio_pid.is_some() && s.equalizer_applied.as_ref() == Some(&settings)
        });
        let state = if live {
            if settings.enabled {
                "ACTIVE · native PCM"
            } else {
                "BYPASSED · native PCM"
            }
        } else if status.is_some_and(|s| s.audio_pid.is_some()) {
            "APPLYING…"
        } else {
            "SAVED · applies on Play"
        };
        let [header, list, hint] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(3),
        ])
        .areas(area);
        f.render_widget(
            Paragraph::new(format!(
                "EQUALIZER · {state}\nAutomatic boost headroom: {:+.1} dB",
                status.map_or(0.0, |s| s.equalizer_headroom_db)
            ))
            .style(Style::default().fg(p.fg)),
            header,
        );
        let slider = |gain: f64| {
            if area.width < 36 {
                return format!("{gain:+5.1} dB");
            }
            let width = usize::from(area.width.saturating_sub(28).clamp(7, 49));
            let position = ((gain + 12.0) / 24.0 * width as f64).round() as usize;
            let bar = (0..=width)
                .map(|i| {
                    if i == position {
                        '●'
                    } else if i == width / 2 {
                        '┼'
                    } else {
                        '─'
                    }
                })
                .collect::<String>();
            format!("[{bar}] {gain:+5.1} dB")
        };
        let preset = (0..PRESETS.len())
            .find(|i| {
                EqSettings::preset(*i).bands_db == settings.bands_db && settings.preamp_db == 0.0
            })
            .map_or("Custom", |i| PRESETS[i]);
        let mut labels = vec![
            format!("Enabled    {}", if settings.enabled { "ON" } else { "OFF" }),
            format!("Preset     {preset} · ←/→ choose"),
        ];
        for (i, frequency) in FREQUENCIES.iter().enumerate() {
            labels.push(format!(
                "{:>6.0} Hz  {}",
                frequency,
                slider(settings.bands_db[i])
            ));
        }
        labels.push(format!("Preamp     {}", slider(settings.preamp_db)));
        let rows = visible_range(self.eq_selected, labels.len(), usize::from(list.height))
            .map(|i| {
                ListItem::new(format!(
                    "{} {}",
                    if i == self.eq_selected { "›" } else { " " },
                    labels[i]
                ))
                .style(if i == self.eq_selected {
                    Style::default().fg(p.bg).bg(p.fg)
                } else {
                    Style::default().fg(p.fg)
                })
            })
            .collect::<Vec<_>>();
        f.render_widget(List::new(rows), list);
        f.render_widget(Paragraph::new("↑/↓ select · ←/→ adjust 0.5 dB · Enter toggle/reset\nb bypass · r Flat · Esc menu · Space play/pause\nBands above the audio Nyquist frequency are bypassed.").style(Style::default().fg(p.muted)),hint);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rapid_eq_edits_accumulate_and_failed_requests_restore_confirmed_settings() {
        let mut app = App::new(Config::default());
        app.screen = 9;
        app.eq_selected = 2;
        app.equalizer_key(KeyCode::Right);
        let request = app.equalizer_key(KeyCode::Right).unwrap();
        let Request::Equalizer(settings) = &request else {
            panic!("missing EQ request")
        };
        assert_eq!(settings.bands_db[0], 1.0);
        app.request_failed(&request);
        assert_eq!(app.eq_settings(), EqSettings::default());
    }
    #[test]
    fn arrows_edit_real_equalizer_gains_and_enter_bypasses_without_erasing_them() {
        let mut app = App::new(Config::default());
        app.screen = 9;
        app.sidebar = false;
        app.eq_selected = 2;
        let request = app
            .equalizer_key(KeyCode::Right)
            .expect("gain edit sends a DSP request");
        let Request::Equalizer(settings) = request else {
            panic!("wrong equalizer command")
        };
        assert_eq!(settings.bands_db[0], 0.5);
        assert!(settings.enabled);
        app.config.equalizer = settings;
        app.eq_selected = 0;
        let Some(Request::Equalizer(settings)) = app.equalizer_key(KeyCode::Enter) else {
            panic!("bypass command missing")
        };
        assert!(!settings.enabled);
        assert_eq!(settings.bands_db[0], 0.5);
    }
}
