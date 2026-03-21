use gtk4::{self as gtk, Box, Orientation, Button, Scale, Label, Adjustment};
use gtk4::prelude::*;
use glib;

use crate::state::SharedState;
use crate::player::PlayerCommand;
use crate::player::RepeatMode;

pub struct PlayerControls {
    root: Box,
    prev_btn: Button,
    play_btn: Button,
    next_btn: Button,
    repeat_btn: Button,
    vol_btn: Button,
    seek_bar: Scale,
    vol_slider: Scale,
    elapsed: Label,
    remaining: Label,
    /// Blocked during programmatic set_value() to avoid feedback loops.
    seek_handler: glib::SignalHandlerId,
}

impl PlayerControls {
    pub fn new(state: SharedState) -> Self {
        let root = Box::builder()
            .orientation(Orientation::Vertical)
            .css_classes(vec!["toolbar", "controls-bar"])
            .build();

        // ── Seek bar ──────────────────────────────────────────────────────
        let seek_adj = Adjustment::new(0.0, 0.0, 1.0, 0.001, 0.01, 0.0);
        let seek_bar = Scale::builder()
            .adjustment(&seek_adj)
            .draw_value(false)
            .hexpand(true)
            .build();
        seek_bar.add_css_class("seekbar");

        // ── Time labels ───────────────────────────────────────────────────
        let time_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .build();
        let elapsed = Label::builder()
            .label("0:00")
            .css_classes(vec!["caption"])
            .build();
        let time_spacer = Box::builder().hexpand(true).build();
        let remaining = Label::builder()
            .label("-0:00")
            .css_classes(vec!["caption"])
            .build();
        time_row.append(&elapsed);
        time_row.append(&time_spacer);
        time_row.append(&remaining);

        // ── Playback buttons ──────────────────────────────────────────────
        let prev_btn = Button::builder()
            .icon_name("media-skip-backward-symbolic")
            .build();
        let play_btn = Button::builder()
            .icon_name("media-playback-start-symbolic")
            .css_classes(vec!["circular", "suggested-action"])
            .build();
        let next_btn = Button::builder()
            .icon_name("media-skip-forward-symbolic")
            .build();

        // ── Repeat ────────────────────────────────────────────────────────
        let repeat_btn = Button::builder()
            .icon_name("media-playlist-repeat-symbolic")
            .css_classes(vec!["repeat-btn"])
            .build();

        // ── Volume ────────────────────────────────────────────────────────
        let vol_btn = Button::builder()
            .icon_name("audio-volume-high-symbolic")
            .build();
        let vol_adj = Adjustment::new(100.0, 0.0, 100.0, 1.0, 10.0, 0.0);
        let vol_slider = Scale::builder()
            .adjustment(&vol_adj)
            .draw_value(false)
            .width_request(90)
            .build();

        // ── Layout ────────────────────────────────────────────────────────
        let end_row = Box::builder()
            .orientation(Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(4)
            .build();

        let center_btns = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .halign(gtk::Align::Center)
            .hexpand(true)
            .build();
        center_btns.append(&prev_btn);
        center_btns.append(&play_btn);
        center_btns.append(&next_btn);

        let vol_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .halign(gtk::Align::End)
            .build();
        vol_box.append(&vol_btn);
        vol_box.append(&vol_slider);

        let left_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        left_box.append(&repeat_btn);

        end_row.append(&left_box);
        end_row.append(&center_btns);
        end_row.append(&vol_box);

        root.append(&seek_bar);
        root.append(&time_row);
        root.append(&end_row);

        // ── Signal: play/pause ────────────────────────────────────────────
        {
            let state_c = state.clone();
            play_btn.connect_clicked(move |_| {
                if let Some(p) = state_c.borrow().player.as_ref() {
                    p.execute(PlayerCommand::TogglePause).ok();
                }
            });
        }

        // ── Signal: prev / next ───────────────────────────────────────────
        {
            let state_c = state.clone();
            prev_btn.connect_clicked(move |_| {
                let mut s = state_c.borrow_mut();
                let new_idx = s.current_idx.and_then(|i| i.checked_sub(1));
                if let Some(idx) = new_idx {
                    if let Some(path) = s.playlist.get(idx).cloned() {
                        s.current_idx = Some(idx);
                        drop(s);
                        if let Some(p) = state_c.borrow().player.as_ref() {
                            p.execute(PlayerCommand::Open(path)).ok();
                        }
                    }
                }
            });
        }
        {
            let state_c = state.clone();
            next_btn.connect_clicked(move |_| {
                let mut s = state_c.borrow_mut();
                let new_idx = s.current_idx.map(|i| i + 1).filter(|&i| i < s.playlist.len());
                if let Some(idx) = new_idx {
                    if let Some(path) = s.playlist.get(idx).cloned() {
                        s.current_idx = Some(idx);
                        drop(s);
                        if let Some(p) = state_c.borrow().player.as_ref() {
                            p.execute(PlayerCommand::Open(path)).ok();
                        }
                    }
                }
            });
        }

        // ── Signal: repeat ───────────────────────────────────────────────
        {
            let state_c = state.clone();
            repeat_btn.connect_clicked(move |_| {
                let next_mode = {
                    let mut s = state_c.borrow_mut();
                    s.repeat_mode = match s.repeat_mode {
                        RepeatMode::None     => RepeatMode::Playlist,
                        RepeatMode::Playlist => RepeatMode::One,
                        RepeatMode::One      => RepeatMode::None,
                    };
                    s.repeat_mode
                };
                if let Some(p) = state_c.borrow().player.as_ref() {
                    p.execute(PlayerCommand::SetRepeat(next_mode)).ok();
                }
            });
        }

        // ── Signal: seek bar ──────────────────────────────────────────────
        // connect_value_changed fires for user drags, scroll, and keyboard on
        // the Scale — but NOT when we call set_value() while the signal is
        // blocked.  mpv discards intermediate seeks during a fast drag, so
        // sending one per event is fine.
        let seek_handler = {
            let state_c = state.clone();
            seek_bar.connect_value_changed(move |scale| {
                let s = state_c.borrow();
                if let Some(p) = s.player.as_ref() {
                    if let Some(dur) = p.duration() {
                        p.execute(PlayerCommand::Seek(scale.value() * dur)).ok();
                    }
                }
            })
        };

        // ── Signal: volume slider ─────────────────────────────────────────
        {
            let state_c = state.clone();
            vol_slider.connect_value_changed(move |scale| {
                if let Some(p) = state_c.borrow().player.as_ref() {
                    p.execute(PlayerCommand::SetVolume(scale.value())).ok();
                }
            });
        }

        // ── Signal: mute ─────────────────────────────────────────────────
        {
            let state_c = state.clone();
            vol_btn.connect_clicked(move |_| {
                let mut s = state_c.borrow_mut();
                s.muted = !s.muted;
                let muted = s.muted;
                if let Some(p) = s.player.as_ref() {
                    p.execute(PlayerCommand::Mute(muted)).ok();
                }
            });
        }

        Self {
            root,
            prev_btn,
            play_btn,
            next_btn,
            repeat_btn,
            vol_btn,
            seek_bar,
            vol_slider,
            elapsed,
            remaining,
            seek_handler,
        }
    }

    pub fn widget(&self) -> &Box {
        &self.root
    }

    /// Called at ~50 ms — only updates the seek bar and time labels.
    pub fn update_position(&self, pos: f64, dur: f64) {
        self.seek_bar.block_signal(&self.seek_handler);
        if dur > 0.0 {
            self.seek_bar.set_value(pos / dur);
        }
        self.seek_bar.unblock_signal(&self.seek_handler);

        self.elapsed.set_label(&format_time(pos as u64));
        if dur > 0.0 {
            self.remaining
                .set_label(&format!("-{}", format_time((dur - pos) as u64)));
        }
    }

    /// Called at ~200 ms — updates buttons and state-driven UI.
    pub fn update(&self, pos: f64, dur: f64, paused: bool, muted: bool, volume: f64, idle: bool, repeat: RepeatMode) {
        let has_media = !idle;
        self.play_btn.set_sensitive(has_media);
        self.prev_btn.set_sensitive(has_media);
        self.next_btn.set_sensitive(has_media);
        self.seek_bar.set_sensitive(has_media);

        self.update_position(pos, dur);

        self.play_btn.set_icon_name(if paused {
            "media-playback-start-symbolic"
        } else {
            "media-playback-pause-symbolic"
        });

        self.vol_btn.set_icon_name(if muted || volume == 0.0 {
            "audio-volume-muted-symbolic"
        } else if volume < 33.0 {
            "audio-volume-low-symbolic"
        } else if volume < 66.0 {
            "audio-volume-medium-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });

        // Repeat button: icon + opacity reflect current mode.
        match repeat {
            RepeatMode::None => {
                self.repeat_btn.set_icon_name("media-playlist-repeat-symbolic");
                self.repeat_btn.set_opacity(0.35);
            }
            RepeatMode::Playlist => {
                self.repeat_btn.set_icon_name("media-playlist-repeat-symbolic");
                self.repeat_btn.set_opacity(1.0);
            }
            RepeatMode::One => {
                self.repeat_btn.set_icon_name("media-playlist-repeat-song-symbolic");
                self.repeat_btn.set_opacity(1.0);
            }
        }
    }
}

fn format_time(total_secs: u64) -> String {
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
