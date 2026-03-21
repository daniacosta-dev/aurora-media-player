use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::{ApplicationWindow, ToolbarView, OverlaySplitView, Breakpoint, BreakpointCondition};
use gtk4::{self as gtk};
use glib;
use gio;
use gdk4;

use crate::library::scan_directory;
use crate::state::{PlayerState, SharedState};
use crate::player::PlayerCommand;

use super::{
    headerbar::MediaHeaderBar,
    video_area::VideoArea,
    controls::PlayerControls,
    playlist::PlaylistPanel,
};

pub struct MediaWindow {
    window: ApplicationWindow,
    state: SharedState,
}

// ── Session persistence ───────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Session {
    playlist: Vec<PathBuf>,
    current_idx: Option<usize>,
    position: f64,
}

fn load_session() -> Option<Session> {
    let path = PlayerState::session_path()?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_session(state: &SharedState, pos: f64) {
    let Some(path) = PlayerState::session_path() else { return };
    let s = state.borrow();
    let session = Session {
        playlist: s.playlist.clone(),
        current_idx: s.current_idx,
        position: pos,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if let Ok(json) = serde_json::to_string_pretty(&session) {
        std::fs::write(path, json).ok();
    }
}

// ── Playlist helpers ──────────────────────────────────────────────────────────

/// Sort `paths` in natural order, update `state.playlist`, and populate the
/// playlist UI.  If `play_first` is true, open the first file automatically.
fn load_playlist(
    paths: Vec<PathBuf>,
    state: &SharedState,
    playlist_ui: &PlaylistPanel,
    play_first: bool,
) {
    let mut paths = paths;
    paths.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
            .cmp(
                &b.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase(),
            )
    });

    {
        let mut s = state.borrow_mut();
        s.playlist = paths.clone();
        s.current_idx = if paths.is_empty() { None } else { Some(0) };
    }

    playlist_ui.clear();
    for path in &paths {
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        playlist_ui.add_item(&title, path);
    }

    if play_first {
        if let Some(path) = paths.first().cloned() {
            if let Some(p) = state.borrow().player.as_ref() {
                p.execute(PlayerCommand::Open(path)).ok();
            }
        }
    }
}

/// Resolve a drop target to a list of media paths (handles files + folders).
fn resolve_drop(path: &Path) -> Vec<PathBuf> {
    if path.is_dir() {
        scan_directory(path)
            .into_iter()
            .map(|item| item.path)
            .collect()
    } else {
        vec![path.to_path_buf()]
    }
}

// ── MediaWindow ───────────────────────────────────────────────────────────────

impl MediaWindow {
    pub fn new(app: &adw::Application) -> Self {
        // ── Shared player state ───────────────────────────────────────────
        let state = PlayerState::create();

        // ── Root window ───────────────────────────────────────────────────
        let window = ApplicationWindow::builder()
            .application(app)
            .title("Aurora Media")
            .default_width(960)
            .default_height(600)
            .width_request(480)
            .height_request(320)
            .build();

        // ── UI components ─────────────────────────────────────────────────
        let header = MediaHeaderBar::new(state.clone());
        let video = VideoArea::new(state.clone());
        let controls = Rc::new(PlayerControls::new(state.clone()));
        let playlist = Rc::new(PlaylistPanel::new(state.clone()));

        // ── Layout ────────────────────────────────────────────────────────
        // Controls float over video so in fullscreen they can hide without
        // shrinking the video area.
        controls.widget().set_valign(gtk::Align::End);
        let video_controls = gtk::Overlay::builder()
            .child(video.widget())
            .hexpand(true)
            .vexpand(true)
            .build();
        video_controls.add_overlay(controls.widget());

        let split_view = OverlaySplitView::builder()
            .sidebar(playlist.widget())
            .content(&video_controls)
            .sidebar_position(gtk::PackType::End)
            .sidebar_width_fraction(0.28)
            .show_sidebar(false)
            .build();

        let toolbar_view = ToolbarView::builder()
            .content(&split_view)
            .build();
        toolbar_view.add_top_bar(header.widget());

        window.set_content(Some(&toolbar_view));

        // ── Playlist toggle ───────────────────────────────────────────────
        header
            .playlist_btn
            .bind_property("active", &split_view, "show-sidebar")
            .sync_create()
            .build();

        // ── Sync maximize button ↔ fullscreen ────────────────────────────
        // When the user clicks the title-bar restore button while fullscreen,
        // detect the unmaximize and also exit fullscreen.
        {
            let window_c = window.downgrade();
            window.connect_notify_local(Some("maximized"), move |_, _| {
                if let Some(win) = window_c.upgrade() {
                    if !win.property::<bool>("maximized") && win.property::<bool>("fullscreened") {
                        win.unfullscreen();
                    }
                }
            });
        }

        // ── Breakpoint: collapse sidebar on narrow windows ────────────────
        let bp = Breakpoint::new(BreakpointCondition::parse("max-width: 720sp").unwrap());
        bp.add_setter(&split_view, "collapsed", &true.to_value());
        window.add_breakpoint(bp);

        // ── Drag & drop ───────────────────────────────────────────────────
        // Accept files AND folders; folders are scanned recursively.
        let drop_target =
            gtk::DropTarget::new(gio::File::static_type(), gdk4::DragAction::COPY);
        {
            let state_c = state.clone();
            let playlist_c = playlist.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                if let Ok(file) = value.get::<gio::File>() {
                    if let Some(path) = file.path() {
                        let paths = resolve_drop(&path);
                        if !paths.is_empty() {
                            load_playlist(paths, &state_c, &playlist_c, true);
                        }
                    }
                }
                true
            });
        }
        window.add_controller(drop_target);

        // ── Click on video: single → play/pause, double → fullscreen ─────
        {
            let window_weak = window.downgrade();
            let state_c = state.clone();
            let dbl_click = gtk::GestureClick::new();
            dbl_click.connect_pressed(move |_, n_press, _, _| {
                if n_press == 1 {
                    if let Some(p) = state_c.borrow().player.as_ref() {
                        p.execute(PlayerCommand::TogglePause).ok();
                    }
                } else if n_press == 2 {
                    // Undo the pause toggle that fired on n_press == 1
                    if let Some(p) = state_c.borrow().player.as_ref() {
                        p.execute(PlayerCommand::TogglePause).ok();
                    }
                    if let Some(win) = window_weak.upgrade() {
                        if win.property::<bool>("fullscreened") {
                            win.unfullscreen();
                            win.unmaximize();
                        } else {
                            win.maximize();
                            win.fullscreen();
                        }
                    }
                }
            });
            video.widget().add_controller(dbl_click);
        }

        // ── Keyboard shortcuts ────────────────────────────────────────────
        {
            let state_c = state.clone();
            let window_weak = window.downgrade();
            let key_ctrl = gtk::EventControllerKey::new();
            key_ctrl.set_propagation_phase(gtk::PropagationPhase::Capture);
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gdk4::Key::space {
                    if let Some(p) = state_c.borrow().player.as_ref() {
                        p.execute(PlayerCommand::TogglePause).ok();
                    }
                } else if key == gdk4::Key::Left {
                    let s = state_c.borrow();
                    if let Some(p) = s.player.as_ref() {
                        if let Some(pos) = p.position() {
                            p.execute(PlayerCommand::Seek((pos - 5.0).max(0.0))).ok();
                        }
                    }
                } else if key == gdk4::Key::Right {
                    let s = state_c.borrow();
                    if let Some(p) = s.player.as_ref() {
                        if let Some(pos) = p.position() {
                            let dur = p.duration().unwrap_or(f64::MAX);
                            p.execute(PlayerCommand::Seek((pos + 5.0).min(dur))).ok();
                        }
                    }
                } else if key == gdk4::Key::f || key == gdk4::Key::F {
                    if let Some(win) = window_weak.upgrade() {
                        if win.property::<bool>("fullscreened") {
                            win.unfullscreen();
                            win.unmaximize();
                        } else {
                            win.maximize();
                            win.fullscreen();
                        }
                    }
                } else if key == gdk4::Key::m || key == gdk4::Key::M {
                    let muted = {
                        let mut s = state_c.borrow_mut();
                        s.muted = !s.muted;
                        s.muted
                    };
                    if let Some(p) = state_c.borrow().player.as_ref() {
                        p.execute(PlayerCommand::Mute(muted)).ok();
                    }
                } else if key == gdk4::Key::Escape {
                    if let Some(win) = window_weak.upgrade() {
                        if win.property::<bool>("fullscreened") {
                            win.unfullscreen();
                            win.unmaximize();
                        }
                    }
                } else {
                    return glib::Propagation::Proceed;
                }
                glib::Propagation::Stop
            });
            window.add_controller(key_ctrl);
        }

        // ── Mouse motion → reset auto-hide timer ──────────────────────────
        // Use a 4-px threshold so sub-pixel jitter from the compositor doesn't
        // keep resetting the timer when the user's mouse is effectively still.
        let last_motion = Rc::new(Cell::new(Instant::now()));
        let last_cursor_pos = Rc::new(Cell::new((-999.0f64, -999.0f64)));
        {
            let last_motion_c = last_motion.clone();
            let last_pos_c = last_cursor_pos.clone();
            let toolbar_view_c = toolbar_view.downgrade();
            let controls_widget_c = controls.widget().downgrade();
            let split_view_c = split_view.downgrade();
            let playlist_btn_c = header.playlist_btn.downgrade();
            let window_c = window.downgrade();
            let motion_ctrl = gtk::EventControllerMotion::new();
            motion_ctrl.connect_motion(move |_, x, y| {
                let (px, py) = last_pos_c.get();
                if (x - px).abs() <= 4.0 && (y - py).abs() <= 4.0 {
                    return; // ignore micro-jitter
                }
                last_pos_c.set((x, y));
                last_motion_c.set(Instant::now());
                if let Some(tv) = toolbar_view_c.upgrade() {
                    tv.set_reveal_top_bars(true);
                }
                if let Some(cw) = controls_widget_c.upgrade() {
                    cw.set_visible(true);
                }
                if let Some(sv) = split_view_c.upgrade() {
                    if let Some(btn) = playlist_btn_c.upgrade() {
                        sv.set_show_sidebar(btn.is_active());
                    }
                }
                if let Some(win) = window_c.upgrade() {
                    win.set_cursor(None::<&gdk4::Cursor>);
                }
            });
            window.add_controller(motion_ctrl);
        }

        // ── Session restore ───────────────────────────────────────────────
        if let Some(session) = load_session() {
            if !session.playlist.is_empty() {
                // Populate the playlist UI without auto-playing.
                let paths = session.playlist;
                {
                    let mut s = state.borrow_mut();
                    s.playlist = paths.clone();
                    s.current_idx = session.current_idx;
                }
                for path in &paths {
                    let title = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("?")
                        .to_string();
                    playlist.add_item(&title, path);
                }
                // Open the last-played file and schedule a seek.
                if let Some(idx) = session.current_idx {
                    if let Some(path) = paths.get(idx).cloned() {
                        playlist.select_row(idx);
                        if let Some(p) = state.borrow().player.as_ref() {
                            p.execute(PlayerCommand::Open(path)).ok();
                        }
                        if session.position > 1.0 {
                            state.borrow_mut().pending_seek = Some(session.position);
                        }
                    }
                }
            }
        }

        // ── Session save on close ─────────────────────────────────────────
        {
            let state_c = state.clone();
            window.connect_close_request(move |_| {
                let pos = state_c
                    .borrow()
                    .player
                    .as_ref()
                    .and_then(|p| p.position())
                    .unwrap_or(0.0);
                save_session(&state_c, pos);
                glib::Propagation::Proceed
            });
        }

        // ── Polling timeout: sync controls, auto-advance, auto-hide ───────
        let window_weak = window.downgrade();
        let toolbar_view_weak = toolbar_view.downgrade();
        let controls_widget_weak = controls.widget().downgrade();
        let split_view_weak = split_view.downgrade();
        let playlist_btn_weak = header.playlist_btn.downgrade();
        let state_c = state.clone();
        let controls_c = controls.clone();
        let playlist_c = playlist.clone();
        let video_c = Rc::new(video);
        // Cooldown counter to avoid double-advancing when eof is briefly still true.
        let advance_cooldown = Rc::new(Cell::new(0u32));

        glib::timeout_add_local(Duration::from_millis(200), move || {
            let (pos, dur, paused, muted, volume, title, idle, has_video,
                 artist, album, eof, pending_seek) = {
                let s = state_c.borrow();
                match s.player.as_ref() {
                    None => return glib::ControlFlow::Continue,
                    Some(p) => (
                        p.position().unwrap_or(0.0),
                        p.duration().unwrap_or(0.0),
                        p.is_paused(),
                        p.is_muted(),
                        p.volume(),
                        p.media_title(),
                        p.is_idle(),
                        p.has_video(),
                        p.metadata_artist().unwrap_or_default(),
                        p.metadata_album().unwrap_or_default(),
                        p.eof_reached(),
                        s.pending_seek,
                    ),
                }
            };

            // ── Pending seek (session restore) ─────────────────────────
            if let Some(seek_to) = pending_seek {
                if dur > 1.0 {
                    if let Some(p) = state_c.borrow().player.as_ref() {
                        p.execute(PlayerCommand::Seek(seek_to)).ok();
                    }
                    state_c.borrow_mut().pending_seek = None;
                }
            }

            // ── Auto-advance to next track ─────────────────────────────
            if advance_cooldown.get() > 0 {
                advance_cooldown.set(advance_cooldown.get() - 1);
            } else if eof && !idle {
                let next = {
                    let s = state_c.borrow();
                    s.current_idx
                        .and_then(|i| {
                            let next = i + 1;
                            s.playlist.get(next).cloned().map(|p| (next, p))
                        })
                };
                if let Some((next_idx, next_path)) = next {
                    state_c.borrow_mut().current_idx = Some(next_idx);
                    playlist_c.select_row(next_idx);
                    if let Some(p) = state_c.borrow().player.as_ref() {
                        p.execute(PlayerCommand::Open(next_path)).ok();
                    }
                    advance_cooldown.set(5); // 5 × 200 ms = 1 s cooldown
                }
            }

            controls_c.update(pos, dur, paused, muted, volume, idle);

            if idle {
                video_c.set_idle(true);
                video_c.show_video();
            } else if has_video {
                video_c.set_idle(false);
                video_c.show_video();
            } else {
                let track_title = title.as_deref().unwrap_or("");
                video_c.show_audio(track_title, &artist, &album);
            }

            if let Some(win) = window_weak.upgrade() {
                win.set_title(title.as_deref().or(Some("Aurora Media Player")));

                if win.property::<bool>("fullscreened") {
                    let idle_secs = last_motion.get().elapsed().as_secs_f64();
                    if idle_secs > 2.0 {
                        if let Some(tv) = toolbar_view_weak.upgrade() {
                            tv.set_reveal_top_bars(false);
                        }
                        if let Some(cw) = controls_widget_weak.upgrade() {
                            cw.set_visible(false);
                        }
                        if let Some(sv) = split_view_weak.upgrade() {
                            sv.set_show_sidebar(false);
                        }
                        win.set_cursor(
                            gdk4::Cursor::from_name("none", None::<&gdk4::Cursor>).as_ref(),
                        );
                    } else {
                        win.set_cursor(None::<&gdk4::Cursor>);
                    }
                } else {
                    if let Some(tv) = toolbar_view_weak.upgrade() {
                        tv.set_reveal_top_bars(true);
                    }
                    if let Some(cw) = controls_widget_weak.upgrade() {
                        cw.set_visible(true);
                    }
                    if let Some(sv) = split_view_weak.upgrade() {
                        if let Some(btn) = playlist_btn_weak.upgrade() {
                            sv.set_show_sidebar(btn.is_active());
                        }
                    }
                    win.set_cursor(None::<&gdk4::Cursor>);
                }
            }

            glib::ControlFlow::Continue
        });

        Self { window, state }
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn open_file(&self, path: &Path) {
        log::info!("Opening from CLI: {:?}", path);
        if let Some(p) = self.state.borrow().player.as_ref() {
            p.execute(PlayerCommand::Open(path.to_path_buf())).ok();
        }
    }
}
