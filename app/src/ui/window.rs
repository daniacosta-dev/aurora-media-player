use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::{ApplicationWindow, Toast, ToastOverlay, ToolbarView, OverlaySplitView, Breakpoint, BreakpointCondition};
use gtk4::{self as gtk};
use glib;
use gio;
use gdk4;

use crate::library::scan_directory;
use crate::mpris::{MprisCommand, MprisState};
use crate::player::{PlayerCommand, RepeatMode};
use crate::state::{PlayerState, SharedState};

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
    // Don't restore to EOF — next open would show last frame frozen.
    let position = if s.player.as_ref().map(|p| p.eof_reached()).unwrap_or(false) {
        0.0
    } else {
        pos
    };
    let session = Session {
        playlist: s.playlist.clone(),
        current_idx: s.current_idx,
        position,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if let Ok(json) = serde_json::to_string_pretty(&session) {
        std::fs::write(path, json).ok();
    }
}

// ── Playlist helpers ──────────────────────────────────────────────────────────

/// Extract a human-readable display title from a path or URL.
/// For URLs: returns the hostname (e.g. "youtube.com") as a loading placeholder.
/// For file paths: returns the file stem (e.g. "My Video").
fn title_for_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")) {
        rest.split('/').next()
            .and_then(|host| host.split('?').next())
            .unwrap_or("URL")
            .to_string()
    } else {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string()
    }
}

/// Parse an M3U or M3U8 playlist file and return a list of (title, path) pairs.
/// Handles `#EXTINF` titles, relative paths, absolute paths, and URLs.
fn parse_m3u(file_path: &Path) -> Vec<(String, PathBuf)> {
    let dir = file_path.parent().unwrap_or(Path::new("."));
    let Ok(content) = std::fs::read_to_string(file_path) else {
        log::warn!("Could not read M3U file: {:?}", file_path);
        return vec![];
    };

    let mut result = Vec::new();
    let mut pending_title: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            // #EXTINF:duration,Title
            if let Some((_, title)) = rest.split_once(',') {
                let title = title.trim();
                if !title.is_empty() {
                    pending_title = Some(title.to_string());
                }
            }
        } else if line.starts_with('#') {
            continue; // skip other comments / directives
        } else {
            let path = if line.starts_with("http://") || line.starts_with("https://") {
                PathBuf::from(line)
            } else {
                let p = Path::new(line);
                if p.is_absolute() { p.to_path_buf() } else { dir.join(p) }
            };
            let title = pending_title.take().unwrap_or_else(|| title_for_path(&path));
            result.push((title, path));
        }
    }
    result
}

/// Returns true if the given path is an M3U/M3U8 playlist.
fn is_m3u(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()).as_deref(),
        Some("m3u") | Some("m3u8")
    )
}

/// Load a pre-built list of (title, path) items into the playlist, in order
/// (no sorting).  If `play_first` is true, open the first item automatically.
fn load_playlist_items(
    items: Vec<(String, PathBuf)>,
    state: &SharedState,
    playlist_ui: &PlaylistPanel,
    play_first: bool,
) {
    {
        let mut s = state.borrow_mut();
        s.playlist = items.iter().map(|(_, p)| p.clone()).collect();
        s.current_idx = if items.is_empty() { None } else { Some(0) };
    }

    playlist_ui.clear();
    for (title, path) in &items {
        playlist_ui.add_item(title, path);
    }

    if play_first {
        if let Some((_, path)) = items.first().cloned() {
            state.borrow_mut().pending_seek = None;
            playlist_ui.select_row(0);
            if let Some(p) = state.borrow().player.as_ref() {
                p.execute(PlayerCommand::Open(path)).ok();
            }
        }
    }
}

/// Sort `paths` in natural order, update `state.playlist`, and populate the
/// playlist UI.  If `play_first` is true, open the first file automatically.
fn load_playlist(
    paths: Vec<PathBuf>,
    state: &SharedState,
    playlist_ui: &PlaylistPanel,
    play_first: bool,
) {
    let mut paths = paths;

    // Sort only file-path playlists; URL playlists preserve user-defined order.
    let is_url_playlist = paths.first()
        .map(|p| { let s = p.to_string_lossy(); s.starts_with("http://") || s.starts_with("https://") })
        .unwrap_or(false);
    if !is_url_playlist {
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
    }

    let items: Vec<(String, PathBuf)> = paths
        .into_iter()
        .map(|p| { let t = title_for_path(&p); (t, p) })
        .collect();
    load_playlist_items(items, state, playlist_ui, play_first);
}

/// Resolve a drop target to a list of (title, path) pairs.
/// Handles files, folders, and M3U playlists.
fn resolve_drop(path: &Path) -> Vec<(String, PathBuf)> {
    if path.is_dir() {
        scan_directory(path)
            .into_iter()
            .map(|item| (title_for_path(&item.path), item.path))
            .collect()
    } else if is_m3u(path) {
        parse_m3u(path)
    } else {
        vec![(title_for_path(path), path.to_path_buf())]
    }
}

// ── MediaWindow ───────────────────────────────────────────────────────────────

impl MediaWindow {
    pub fn new(app: &adw::Application) -> Self {
        // ── MPRIS server (background thread) ──────────────────────────────
        let (mpris, mpris_cmd_rx) = crate::mpris::spawn();
        let mpris = Rc::new(mpris);
        let mpris_cmd_rx = Rc::new(mpris_cmd_rx);

        // ── Shared player state ───────────────────────────────────────────
        let state = PlayerState::create();

        // ── Restore global volume/mute from settings ──────────────────────
        {
            let settings = super::headerbar::load_app_settings();
            if let Some(p) = state.borrow().player.as_ref() {
                p.execute(PlayerCommand::SetVolume(settings.volume)).ok();
                p.execute(PlayerCommand::Mute(settings.muted)).ok();
            }
            state.borrow_mut().muted = settings.muted;
        }

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
        // Playlist is created first so it can be referenced in the header callback.
        let playlist = Rc::new(PlaylistPanel::new(state.clone()));
        let video = VideoArea::new(state.clone());
        let controls = Rc::new(PlayerControls::new(state.clone()));
        let header = {
            let state_file = state.clone();
            let playlist_file = playlist.clone();
            let state_url = state.clone();
            let playlist_url = playlist.clone();
            let state_sub = state.clone();
            MediaHeaderBar::new(
                state.clone(),
                move |path: PathBuf| {
                    if is_m3u(&path) {
                        let items = parse_m3u(&path);
                        if !items.is_empty() {
                            load_playlist_items(items, &state_file, &playlist_file, true);
                        }
                    } else {
                        state_file.borrow_mut().pending_seek = None;
                        if let Some(p) = state_file.borrow().player.as_ref() {
                            if let Err(e) = p.execute(PlayerCommand::Open(path)) {
                                log::error!("open file: {e}");
                            }
                        }
                    }
                },
                move |items: Vec<(String, String)>| {
                    let items: Vec<(String, PathBuf)> = items
                        .into_iter()
                        .map(|(t, u)| (t, PathBuf::from(u)))
                        .collect();
                    state_url.borrow_mut().pending_seek = None;
                    load_playlist_items(items, &state_url, &playlist_url, true);
                },
                move |path: PathBuf| {
                    if let Some(p) = state_sub.borrow().player.as_ref() {
                        p.execute(PlayerCommand::AddSubtitle(path)).ok();
                    }
                },
                {
                    let state_recent = state.clone();
                    let playlist_recent = playlist.clone();
                    move |path: PathBuf| {
                        if is_m3u(&path) {
                            let items = parse_m3u(&path);
                            if !items.is_empty() {
                                load_playlist_items(items, &state_recent, &playlist_recent, true);
                            }
                        } else {
                            state_recent.borrow_mut().pending_seek = None;
                            if let Some(p) = state_recent.borrow().player.as_ref() {
                                p.execute(PlayerCommand::Open(path)).ok();
                            }
                        }
                    }
                },
            )
        };

        let push_recent = header.push_recent_fn.clone();

        // ── Layout ────────────────────────────────────────────────────────
        // Controls float over video so in fullscreen they can hide without
        // shrinking the video area.
        controls.widget().set_valign(gtk::Align::End);
        let controls_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(220)
            .reveal_child(true)
            .valign(gtk::Align::End)
            .child(controls.widget())
            .build();
        let video_controls = gtk::Overlay::builder()
            .child(video.widget())
            .hexpand(true)
            .vexpand(true)
            .build();
        video_controls.add_overlay(&controls_revealer);

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

        let toast_overlay = ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar_view));
        window.set_content(Some(&toast_overlay));

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
                        let items = resolve_drop(&path);
                        if !items.is_empty() {
                            load_playlist_items(items, &state_c, &playlist_c, true);
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

        // ── Track whether mouse is over the controls bar ──────────────────
        let mouse_over_controls: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        {
            let moc_enter = mouse_over_controls.clone();
            let moc_leave = mouse_over_controls.clone();
            let mc = gtk::EventControllerMotion::new();
            mc.connect_enter(move |_, _, _| { moc_enter.set(true); });
            mc.connect_leave(move |_| { moc_leave.set(false); });
            controls_revealer.add_controller(mc);
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
            let controls_revealer_c = controls_revealer.downgrade();
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
                if let Some(r) = controls_revealer_c.upgrade() {
                    r.set_reveal_child(true);
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
            // Grab the previously-playing path before consuming the vec.
            let prev_current = session.current_idx.and_then(|i| session.playlist.get(i).cloned());
            // Drop files that were moved or deleted since last run.
            let paths: Vec<PathBuf> = session.playlist.into_iter().filter(|p| p.exists()).collect();
            if !paths.is_empty() {
                // Find the new index of the previously-playing file; fall back to 0.
                let new_idx = prev_current
                    .as_ref()
                    .and_then(|orig| paths.iter().position(|p| p == orig))
                    .or(Some(0));
                {
                    let mut s = state.borrow_mut();
                    s.playlist = paths.clone();
                    s.current_idx = new_idx;
                }
                for path in &paths {
                    let title = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("?")
                        .to_string();
                    playlist.add_item(&title, path);
                }
                // Defer opening the file until the GL render context is ready.
                // With vo=libmpv, loadfile before the render context causes mpv
                // to fail video output init and stay idle.
                if let Some(idx) = new_idx {
                    if let Some(path) = paths.get(idx).cloned() {
                        playlist.select_row(idx);
                        state.borrow_mut().pending_open = Some(path);
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

        // ── Fast position timer: seek bar + time labels at 50 ms ─────────
        {
            let state_c = state.clone();
            let controls_c = controls.clone();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let s = state_c.borrow();
                if let Some(p) = s.player.as_ref() {
                    let pos = p.position().unwrap_or(0.0);
                    let dur = p.duration().unwrap_or(0.0);
                    drop(s);
                    controls_c.update_position(pos, dur);
                }
                glib::ControlFlow::Continue
            });
        }

        // ── Polling timeout: sync controls, auto-advance, auto-hide ───────
        let window_weak = window.downgrade();
        let toolbar_view_weak = toolbar_view.downgrade();
        let controls_revealer_weak = controls_revealer.downgrade();
        let mouse_over_controls_c = mouse_over_controls.clone();
        let split_view_weak = split_view.downgrade();
        let playlist_btn_weak = header.playlist_btn.downgrade();
        let state_c = state.clone();
        let controls_c = controls.clone();
        let playlist_c = playlist.clone();
        let video_c = Rc::new(video);
        let mpris_c = mpris.clone();
        let mpris_cmd_rx_c = mpris_cmd_rx.clone();
        let toast_overlay_c = toast_overlay.clone();
        let push_recent_c = push_recent.clone();
        // Cooldown counter to avoid double-advancing when eof is briefly still true.
        let advance_cooldown = Rc::new(Cell::new(0u32));
        // Track previous idle state to detect unexpected stops (playback errors).
        let prev_idle = Rc::new(Cell::new(true));
        // Track volume/mute to persist changes to global settings.
        let last_saved_volume = Rc::new(Cell::new(super::headerbar::load_app_settings().volume));
        let last_saved_muted = Rc::new(Cell::new(super::headerbar::load_app_settings().muted));
        // For URL items, defer the recent-files push until yt-dlp resolves the real title.
        let recent_pending_idx: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        // Track current_idx so any external change (prev/next buttons) syncs the playlist UI.
        let last_known_idx: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));

        glib::timeout_add_local(Duration::from_millis(200), move || {
            let (pos, dur, paused, muted, volume, speed, title, idle, has_video,
                 artist, album, eof, pending_seek, repeat_mode, shuffle) = {
                let s = state_c.borrow();
                match s.player.as_ref() {
                    None => return glib::ControlFlow::Continue,
                    Some(p) => (
                        p.position().unwrap_or(0.0),
                        p.duration().unwrap_or(0.0),
                        p.is_paused(),
                        p.is_muted(),
                        p.volume(),
                        p.speed(),
                        p.media_title(),
                        p.is_idle(),
                        p.has_video(),
                        p.metadata_artist().unwrap_or_default(),
                        p.metadata_album().unwrap_or_default(),
                        p.eof_reached(),
                        s.pending_seek,
                        s.repeat_mode,
                        s.shuffle,
                    ),
                }
            };

            // ── Playback error detection ────────────────────────────────
            // If playback was active and suddenly becomes idle (not due to
            // EOF), mpv likely failed to open the source. Show a toast.
            let was_idle = prev_idle.get();
            if !was_idle && idle && !eof {
                let err_msg = state_c
                    .borrow()
                    .player
                    .as_ref()
                    .and_then(|p| p.last_error())
                    .unwrap_or_else(|| "Could not open the media source.".into());
                toast_overlay_c.add_toast(Toast::new(&err_msg));
            }
            prev_idle.set(idle);

            // ── Persist volume/mute to global settings when they change ──
            if (volume - last_saved_volume.get()).abs() > 0.5 || muted != last_saved_muted.get() {
                last_saved_volume.set(volume);
                last_saved_muted.set(muted);
                let mut s = super::headerbar::load_app_settings();
                s.volume = volume;
                s.muted = muted;
                super::headerbar::save_app_settings(&s);
            }

            // ── Push to recent files when playback starts ───────────────
            // Local files: push immediately (title is known from the filename).
            // URL items: mark as pending — defer until yt-dlp resolves the title.
            if was_idle && !idle {
                if let Some(idx) = state_c.borrow().current_idx {
                    let path = state_c.borrow().playlist.get(idx).cloned();
                    if let Some(ref path) = path {
                        let path_str = path.to_string_lossy();
                        if path_str.starts_with("http://") || path_str.starts_with("https://") {
                            recent_pending_idx.set(Some(idx));
                        } else {
                            let display_title = title.as_deref()
                                .unwrap_or_else(|| path.file_stem().and_then(|s| s.to_str()).unwrap_or("?"));
                            push_recent_c(path, display_title);
                        }
                    }
                }
            }

            // If a pending URL item stops playing before the title resolved, save what we have.
            if idle && !was_idle {
                if let Some(pending) = recent_pending_idx.get() {
                    let path = state_c.borrow().playlist.get(pending).cloned();
                    if let Some(path) = path {
                        let best = playlist_c.item_title(pending)
                            .unwrap_or_else(|| title_for_path(&path));
                        push_recent_c(&path, &best);
                    }
                    recent_pending_idx.set(None);
                }
            }

            // ── Sync playlist selection with current_idx ────────────────
            // Handles prev/next button clicks in controls.rs which update
            // state.current_idx but don't have access to PlaylistPanel.
            {
                let cur_idx = state_c.borrow().current_idx;
                if cur_idx != last_known_idx.get() {
                    if let Some(idx) = cur_idx {
                        playlist_c.select_row(idx);
                    }
                    last_known_idx.set(cur_idx);
                }
            }

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
                            s.effective_next_idx(i).and_then(|next| {
                                s.playlist.get(next).cloned().map(|p| (next, p))
                            })
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

            controls_c.update(pos, dur, paused, muted, volume, speed, idle, has_video, repeat_mode, shuffle);

            // ── Tracks (audio + subtitles) ──────────────────────────────
            let tracks = {
                let s = state_c.borrow();
                s.player.as_ref().map(|p| p.track_list()).unwrap_or_default()
            };
            controls_c.update_tracks(tracks, &state_c);

            // ── Chapter marks on seek bar ───────────────────────────────
            let chapters = {
                let s = state_c.borrow();
                s.player.as_ref().filter(|_| !idle)
                    .map(|p| p.chapter_list())
                    .unwrap_or_default()
            };
            controls_c.update_chapters(chapters, dur);

            // ── Update URL playlist row title once mpv/yt-dlp resolves it ─
            // Only replace if the title is still the hostname fallback — don't
            // clobber a proper #EXTINF channel name with mpv stream metadata.
            if !idle {
                if let (Some(real_title), Some(idx)) = (title.as_deref(), state_c.borrow().current_idx) {
                    let still_placeholder = state_c.borrow().playlist.get(idx)
                        .and_then(|p| {
                            let path_str = p.to_string_lossy();
                            // Only applies to URL items — file items don't need title updates.
                            if !path_str.starts_with("http://") && !path_str.starts_with("https://") {
                                return None;
                            }
                            let fallback = title_for_path(p);
                            playlist_c.item_title(idx).map(|t| {
                                // Still a placeholder if:
                                // 1. hostname fallback ("youtube.com")
                                // 2. mpv returned the raw full URL before yt-dlp resolved
                                // 3. mpv returned just the URL path fragment ("watch?v=xxx")
                                //    which has no spaces and contains a query-string marker
                                // Real titles (from yt-dlp) have spaces and no URL syntax.
                                t == fallback
                                    || t.starts_with("http://")
                                    || t.starts_with("https://")
                                    || (t.contains('?') && !t.contains(' '))
                            })
                        })
                        .unwrap_or(false);
                    if still_placeholder {
                        playlist_c.update_row_title(idx, real_title);

                        // If this is the real resolved title (not another URL fragment),
                        // push to recent now and clear the pending flag.
                        let title_is_real = !real_title.starts_with("http://")
                            && !real_title.starts_with("https://")
                            && !(real_title.contains('?') && !real_title.contains(' '));
                        if title_is_real && recent_pending_idx.get() == Some(idx) {
                            if let Some(path) = state_c.borrow().playlist.get(idx).cloned() {
                                push_recent_c(&path, real_title);
                                recent_pending_idx.set(None);
                            }
                        }
                    }
                }
            }

            if idle {
                video_c.set_idle(true);
                video_c.set_audio_playing(false);
                video_c.show_video();
            } else if has_video {
                video_c.set_idle(false);
                video_c.set_audio_playing(false);
                video_c.show_video();
            } else {
                let track_title = title.as_deref().unwrap_or("");
                video_c.show_audio(track_title, &artist, &album);
                video_c.set_audio_playing(!paused);
            }

            if let Some(win) = window_weak.upgrade() {
                win.set_title(Some("Aurora Media Player"));

                let idle_secs = last_motion.get().elapsed().as_secs_f64();
                let hide_after = if mouse_over_controls_c.get() { 5.0 } else { 2.0 };

                if win.property::<bool>("fullscreened") {
                    if idle_secs > hide_after {
                        if let Some(tv) = toolbar_view_weak.upgrade() {
                            tv.set_reveal_top_bars(false);
                        }
                        if let Some(r) = controls_revealer_weak.upgrade() {
                            r.set_reveal_child(false);
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
                    // Header always visible outside fullscreen (it's the window chrome).
                    if let Some(tv) = toolbar_view_weak.upgrade() {
                        tv.set_reveal_top_bars(true);
                    }
                    // Hide bottom controls after 2s (or 5s if mouse is over controls).
                    if idle_secs > hide_after && !idle {
                        if let Some(r) = controls_revealer_weak.upgrade() {
                            r.set_reveal_child(false);
                        }
                    } else {
                        if let Some(r) = controls_revealer_weak.upgrade() {
                            r.set_reveal_child(true);
                        }
                        if let Some(sv) = split_view_weak.upgrade() {
                            if let Some(btn) = playlist_btn_weak.upgrade() {
                                sv.set_show_sidebar(btn.is_active());
                            }
                        }
                    }
                    win.set_cursor(None::<&gdk4::Cursor>);
                }
            }

            // ── MPRIS: drain incoming commands ─────────────────────────
            while let Ok(cmd) = mpris_cmd_rx_c.try_recv() {
                match cmd {
                    MprisCommand::PlayPause => {
                        if let Some(p) = state_c.borrow().player.as_ref() {
                            p.execute(PlayerCommand::TogglePause).ok();
                        }
                    }
                    MprisCommand::Next => {
                        let next = {
                            let s = state_c.borrow();
                            s.current_idx.and_then(|i| {
                                s.effective_next_idx(i).and_then(|ni| s.playlist.get(ni).cloned().map(|p| (ni, p)))
                            })
                        };
                        if let Some((idx, path)) = next {
                            state_c.borrow_mut().current_idx = Some(idx);
                            playlist_c.select_row(idx);
                            if let Some(p) = state_c.borrow().player.as_ref() {
                                p.execute(PlayerCommand::Open(path)).ok();
                            }
                            advance_cooldown.set(5);
                        }
                    }
                    MprisCommand::Previous => {
                        let prev = {
                            let s = state_c.borrow();
                            s.current_idx.and_then(|i| {
                                if i > 0 {
                                    s.playlist.get(i - 1).cloned().map(|p| (i - 1, p))
                                } else {
                                    None
                                }
                            })
                        };
                        if let Some((idx, path)) = prev {
                            state_c.borrow_mut().current_idx = Some(idx);
                            playlist_c.select_row(idx);
                            if let Some(p) = state_c.borrow().player.as_ref() {
                                p.execute(PlayerCommand::Open(path)).ok();
                            }
                            advance_cooldown.set(5);
                        }
                    }
                    MprisCommand::Seek(offset_us) => {
                        let s = state_c.borrow();
                        if let Some(p) = s.player.as_ref() {
                            let new_pos = (p.position().unwrap_or(0.0)
                                + offset_us as f64 / 1_000_000.0)
                                .max(0.0)
                                .min(p.duration().unwrap_or(f64::MAX));
                            p.execute(PlayerCommand::Seek(new_pos)).ok();
                        }
                    }
                    MprisCommand::SetVolume(v) => {
                        if let Some(p) = state_c.borrow().player.as_ref() {
                            p.execute(PlayerCommand::SetVolume(v * 100.0)).ok();
                        }
                    }
                }
            }

            // ── MPRIS: push current state ───────────────────────────────
            {
                let (can_next, can_prev) = {
                    let s = state_c.borrow();
                    let n = s.playlist.len();
                    let i = s.current_idx.unwrap_or(0);
                    (i + 1 < n, i > 0 && n > 0)
                };
                let playback_status = if idle {
                    "Stopped"
                } else if paused {
                    "Paused"
                } else {
                    "Playing"
                };
                let loop_status = match repeat_mode {
                    RepeatMode::None => "None",
                    RepeatMode::One => "Track",
                    RepeatMode::Playlist => "Playlist",
                };
                mpris_c.update(MprisState {
                    playback_status: playback_status.into(),
                    loop_status: loop_status.into(),
                    title: title.as_deref().unwrap_or("").into(),
                    artist: artist.clone(),
                    album: album.clone(),
                    position_us: (pos * 1_000_000.0) as i64,
                    duration_us: (dur * 1_000_000.0) as i64,
                    volume: volume / 100.0,
                    can_go_next: can_next,
                    can_go_previous: can_prev,
                });
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
        // Cancel any pending session seek — it belongs to the restored file, not this one.
        self.state.borrow_mut().pending_seek = None;
        if let Some(p) = self.state.borrow().player.as_ref() {
            p.execute(PlayerCommand::Open(path.to_path_buf())).ok();
        }
    }
}
