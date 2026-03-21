use std::rc::Rc;
use std::path::Path;
use std::time::Duration;

use adw::prelude::*;
use adw::{ApplicationWindow, ToolbarView, OverlaySplitView, Breakpoint, BreakpointCondition};
use gtk4::{self as gtk, Box, Orientation};
use glib;
use gio;
use gdk4;

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

        // ── UI components (each wire their own signals against `state`) ───
        let header = MediaHeaderBar::new(state.clone());
        let video = VideoArea::new(state.clone());
        let controls = Rc::new(PlayerControls::new(state.clone()));
        let playlist = PlaylistPanel::new(state.clone());

        // ── Layout ────────────────────────────────────────────────────────
        let content = Box::new(Orientation::Vertical, 0);
        content.append(video.widget());
        content.append(controls.widget());

        let split_view = OverlaySplitView::builder()
            .sidebar(playlist.widget())
            .content(&content)
            .sidebar_position(gtk::PackType::End)
            .sidebar_width_fraction(0.28)
            .show_sidebar(false)
            .build();

        let toolbar_view = ToolbarView::builder()
            .content(&split_view)
            .build();
        toolbar_view.add_top_bar(header.widget());

        window.set_content(Some(&toolbar_view));

        // ── Bind playlist toggle → sidebar visibility ─────────────────────
        header
            .playlist_btn
            .bind_property("active", &split_view, "show-sidebar")
            .sync_create()
            .build();

        // ── Breakpoint: collapse sidebar on small windows ─────────────────
        let bp =
            Breakpoint::new(BreakpointCondition::parse("max-width: 720sp").unwrap());
        bp.add_setter(&split_view, "collapsed", &true.to_value());
        window.add_breakpoint(bp);

        // ── Drag & drop ───────────────────────────────────────────────────
        let drop_target =
            gtk::DropTarget::new(gio::File::static_type(), gdk4::DragAction::COPY);
        {
            let state_c = state.clone();
            drop_target.connect_drop(move |_, value, _, _| {
                if let Ok(file) = value.get::<gio::File>() {
                    if let Some(path) = file.path() {
                        Self::open_path(&state_c, &path);
                    }
                }
                true
            });
        }
        window.add_controller(drop_target);

        // ── Polling timeout: sync controls with mpv every 200 ms ──────────
        // Keep a weak reference to the window for the title update so the
        // timeout doesn't prevent the window from being freed.
        let window_weak = window.downgrade();
        let state_c = state.clone();
        let controls_c = controls.clone();
        let video_c = Rc::new(video);

        glib::timeout_add_local(Duration::from_millis(200), move || {
            let (pos, dur, paused, muted, volume, title, idle, has_video, artist, album) = {
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
                    ),
                }
            };

            controls_c.update(pos, dur, paused, muted, volume);

            if idle {
                video_c.set_idle(true);
                video_c.show_video();
            } else if has_video {
                video_c.set_idle(false);
                video_c.show_video();
            } else {
                // Audio-only file: show the metadata card.
                let track_title = title.as_deref().unwrap_or("");
                video_c.show_audio(track_title, &artist, &album);
            }

            if let Some(win) = window_weak.upgrade() {
                win.set_title(title.as_deref().or(Some("Aurora Media")));
            }

            glib::ControlFlow::Continue
        });

        Self { window, state }
    }

    pub fn widget(&self) -> &ApplicationWindow {
        &self.window
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn open_file(&self, path: &Path) {
        Self::open_path(&self.state, path);
    }

    fn open_path(state: &SharedState, path: &Path) {
        log::info!("Opening: {:?}", path);
        if let Some(p) = state.borrow().player.as_ref() {
            p.execute(PlayerCommand::Open(path.to_path_buf())).ok();
        }
    }
}
