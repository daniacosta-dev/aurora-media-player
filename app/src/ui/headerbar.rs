use adw::HeaderBar;
use gtk4::{self as gtk, Button, MenuButton, ToggleButton};
use gtk4::prelude::*;
use gio;

use crate::state::SharedState;
use crate::player::PlayerCommand;

pub struct MediaHeaderBar {
    header: HeaderBar,
    /// Exposed so the window can bind it to the OverlaySplitView's
    /// `show-sidebar` property.
    pub playlist_btn: ToggleButton,
}

impl MediaHeaderBar {
    pub fn new(state: SharedState) -> Self {
        let header = HeaderBar::new();

        // ── Open file button ──────────────────────────────────────────────
        let open_btn = Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Open file")
            .build();
        header.pack_start(&open_btn);

        // ── Playlist toggle ───────────────────────────────────────────────
        let playlist_btn = ToggleButton::builder()
            .icon_name("view-list-symbolic")
            .tooltip_text("Toggle playlist")
            .build();
        header.pack_end(&playlist_btn);

        // ── Menu button ───────────────────────────────────────────────────
        let menu_btn = MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Menu")
            .build();
        header.pack_end(&menu_btn);

        // ── Wire: open button → GTK FileDialog (portal-backed) ────────────
        // gtk::FileDialog uses xdg-desktop-portal automatically, making it
        // compatible with strict Snap confinement.
        {
            let state_c = state.clone();
            open_btn.connect_clicked(move |btn| {
                // Build filter for all supported media formats.
                let media_filter = gtk::FileFilter::new();
                media_filter.set_name(Some("Media files"));
                for ext in [
                    "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "ts",
                    "mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wma",
                ] {
                    media_filter.add_suffix(ext);
                }

                let video_filter = gtk::FileFilter::new();
                video_filter.set_name(Some("Video files"));
                for ext in ["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "ts"] {
                    video_filter.add_suffix(ext);
                }

                let audio_filter = gtk::FileFilter::new();
                audio_filter.set_name(Some("Audio files"));
                for ext in ["mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wma"] {
                    audio_filter.add_suffix(ext);
                }

                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&media_filter);
                filters.append(&video_filter);
                filters.append(&audio_filter);

                let dialog = gtk::FileDialog::builder()
                    .title("Open Media File")
                    .modal(true)
                    .filters(&filters)
                    .build();

                // Resolve the parent window at click-time so we don't hold a
                // strong reference to it inside the button's closure.
                let parent = btn.root().and_downcast::<gtk::Window>();

                let state_inner = state_c.clone();
                dialog.open(
                    parent.as_ref(),
                    None::<&gio::Cancellable>,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                // User chose a new file — cancel any session-restore seek.
                                state_inner.borrow_mut().pending_seek = None;
                                if let Some(p) = state_inner.borrow().player.as_ref() {
                                    if let Err(e) = p.execute(PlayerCommand::Open(path)) {
                                        log::error!("open file: {e}");
                                    }
                                } else {
                                    log::error!("open file: player not initialized");
                                }
                            }
                        }
                    },
                );
            });
        }

        Self {
            header,
            playlist_btn,
        }
    }

    pub fn widget(&self) -> &HeaderBar {
        &self.header
    }
}
