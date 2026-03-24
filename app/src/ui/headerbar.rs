use std::path::PathBuf;
use std::rc::Rc;
use std::cell::RefCell;
use std::time::Duration;

use adw::{self, HeaderBar};
use adw::prelude::*;
use gtk4::{self as gtk, Button, ToggleButton};
use gtk4::prelude::*;
use gio;

use crate::state::SharedState;

// ── Recent files persistence ──────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct RecentEntry {
    path: String,
    title: String,
}

fn recent_path() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("aurora-media").join("recent.json"))
}

fn load_recent() -> Vec<RecentEntry> {
    let Some(path) = recent_path() else { return vec![] };
    let Ok(data) = std::fs::read_to_string(path) else { return vec![] };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_recent(entries: &[RecentEntry]) {
    let Some(path) = recent_path() else { return };
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).ok(); }
    if let Ok(json) = serde_json::to_string_pretty(entries) {
        std::fs::write(path, json).ok();
    }
}

pub struct MediaHeaderBar {
    header: HeaderBar,
    pub playlist_btn: ToggleButton,
    pub push_recent_fn: Rc<dyn Fn(&std::path::Path, &str)>,
    pub window_title: adw::WindowTitle,
    /// Exposed for keyboard shortcuts — activate() triggers the connected handler.
    pub open_file_btn: Button,
    pub open_url_btn: Button,
    pub open_sub_btn: Button,
    pub settings_btn: Button,
}

impl MediaHeaderBar {
    /// `on_open_file` is called when the user picks a file via the open dialog.
    /// The callback receives the path and is responsible for handling both
    /// regular media files and M3U playlists (wired in window.rs).
    ///
    /// `on_url_playlist` is called when the user confirms a URL playlist.
    /// Items are `(display_title, url)` pairs — titles come from `#EXTINF` when available.
    ///
    /// `on_open_subtitle` is called when the user picks a subtitle file.
    ///
    /// `on_open_recent` is called when the user clicks a recent file entry.
    pub fn new(
        _state: SharedState,
        on_open_file: impl Fn(PathBuf) + 'static,
        on_url_playlist: impl Fn(Vec<(String, String)>) + 'static,
        on_open_subtitle: impl Fn(PathBuf) + 'static,
        on_open_recent: impl Fn(PathBuf) + 'static,
        on_ui_mode_change: Rc<dyn Fn(&str)>,
    ) -> Self {
        let header = HeaderBar::new();

        // ── File menu ─────────────────────────────────────────────────────────────
        let file_btn = Button::builder().label("File").build();

        // Main popover — autohide closes it when clicking outside
        let file_popover = gtk::Popover::new();
        file_popover.set_autohide(true);
        file_popover.set_has_arrow(false);
        file_popover.add_css_class("file-menu-popover");
        file_popover.set_parent(&file_btn);

        let menu_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(4).margin_bottom(4)
            .build();
        file_popover.set_child(Some(&menu_box));

        // Helper: flat button styled as a menu item
        let mk_item = |icon: &str, label: &str| -> Button {
            let btn = Button::new();
            btn.add_css_class("flat");
            btn.add_css_class("file-menu-item");
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_start(8).margin_end(16)
                .margin_top(2).margin_bottom(2)
                .build();
            row.append(&gtk::Image::from_icon_name(icon));
            let lbl = gtk::Label::builder()
                .label(label)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .build();
            row.append(&lbl);
            btn.set_child(Some(&row));
            btn
        };

        let open_file_btn = mk_item("document-open-symbolic",       "Open File…");
        let open_url_btn  = mk_item("insert-link-symbolic",          "Open URL or Playlist…");
        let open_sub_btn  = mk_item("media-view-subtitles-symbolic", "Load Subtitle File…");

        // Recent Files row — right-arrow indicates a submenu
        let recent_row_btn = {
            let btn = Button::new();
            btn.add_css_class("flat");
            btn.add_css_class("file-menu-item");
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_start(8).margin_end(16)
                .margin_top(2).margin_bottom(2)
                .build();
            row.append(&gtk::Image::from_icon_name("document-open-recent-symbolic"));
            let lbl = gtk::Label::builder()
                .label("Recent Files")
                .halign(gtk::Align::Start)
                .hexpand(true)
                .build();
            row.append(&lbl);
            row.append(&gtk::Image::from_icon_name("go-next-symbolic"));
            btn.set_child(Some(&row));
            btn
        };

        let screenshot_folder_btn = mk_item("camera-photo-symbolic",  "Open Screenshot Folder");
        let report_issue_btn      = mk_item("bug-symbolic",            "Report Issue");
        report_issue_btn.set_cursor_from_name(Some("pointer"));

        menu_box.append(&open_file_btn);
        menu_box.append(&open_url_btn);
        menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu_box.append(&open_sub_btn);
        menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu_box.append(&recent_row_btn);
        menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu_box.append(&screenshot_folder_btn);
        menu_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu_box.append(&report_issue_btn);

        // ── Recent sub-popover ─────────────────────────────────────────────────
        // Parented to recent_row_btn (inside file_popover) so autohide on the
        // main popover correctly considers clicks here as "inside the cascade".
        let recent_sub = gtk::Popover::new();
        recent_sub.set_autohide(false);
        recent_sub.set_has_arrow(false);
        recent_sub.add_css_class("file-menu-popover");
        recent_sub.set_parent(&recent_row_btn);
        recent_sub.set_position(gtk::PositionType::Right);

        let recent_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(4).margin_bottom(4)
            .build();
        recent_sub.set_child(Some(&recent_box));

        // ── Hover / auto-close logic ───────────────────────────────────────────
        // - Main popover: stays open; closes on click outside or click "File".
        // - Recent sub:   opens on hover over "Recent Files" row;
        //                 closes only when hovering another menu item.

        // Hover Recent Files row → popup sub
        {
            let rs = recent_sub.downgrade();
            let mc = gtk::EventControllerMotion::new();
            mc.connect_enter(move |_, _, _| {
                if let Some(r) = rs.upgrade() { r.popup(); }
            });
            recent_row_btn.add_controller(mc);
        }

        // Hover any other item → close sub
        for btn in [&open_file_btn, &open_url_btn, &open_sub_btn, &screenshot_folder_btn, &report_issue_btn] {
            let rs = recent_sub.downgrade();
            let mc = gtk::EventControllerMotion::new();
            mc.connect_enter(move |_, _, _| {
                if let Some(r) = rs.upgrade() { r.popdown(); }
            });
            btn.add_controller(mc);
        }

        // Report Issue → open GitHub issues page in the default browser
        {
            let fp = file_popover.downgrade();
            report_issue_btn.connect_clicked(move |_| {
                if let Some(f) = fp.upgrade() { f.popdown(); }
                gio::AppInfo::launch_default_for_uri(
                    "https://github.com/daniacosta-dev/aurora-media-player/issues",
                    gio::AppLaunchContext::NONE,
                ).ok();
            });
        }

        // Main popover closed → also close sub
        {
            let rs = recent_sub.downgrade();
            file_popover.connect_closed(move |_| {
                if let Some(r) = rs.upgrade() { r.popdown(); }
            });
        }

        // File button click → toggle main popover
        {
            let fp = file_popover.downgrade();
            file_btn.connect_clicked(move |_| {
                if let Some(f) = fp.upgrade() {
                    if f.is_visible() { f.popdown(); } else { f.popup(); }
                }
            });
        }

        // Align left edge of popover with the window edge
        {
            let btn_w = file_btn.downgrade();
            file_popover.connect_show(move |popover| {
                if let Some(btn) = btn_w.upgrade() {
                    if let Some(root) = btn.root() {
                        if let Some((x, _)) = btn.translate_coordinates(&root, 0.0, 0.0) {
                            let btn_center = x + btn.width() as f64 / 2.0;
                            // 140 = half of CSS min-width 280px
                            let offset = ((140.0 - btn_center) as i32).max(0);
                            popover.set_offset(offset, 0);
                        }
                    }
                }
            });
        }

        header.pack_start(&file_btn);

        // ── Window title (centered, title + subtitle) ─────────────────────
        let window_title = adw::WindowTitle::builder()
            .title("Aurora Media Player")
            .build();
        header.set_title_widget(Some(&window_title));

        // ── Playlist toggle ───────────────────────────────────────────────
        let playlist_btn = ToggleButton::builder()
            .icon_name("view-list-symbolic")
            .tooltip_text("Toggle playlist")
            .build();
        header.pack_end(&playlist_btn);

        // ── Settings button ───────────────────────────────────────────────
        let settings_btn = Button::builder()
            .icon_name("preferences-system-symbolic")
            .tooltip_text("Settings")
            .build();
        header.pack_end(&settings_btn);

        // Apply persisted color scheme immediately on construction.
        let saved_scheme = load_app_settings()
            .color_scheme
            .unwrap_or_else(|| "system".into());
        adw::StyleManager::default().set_color_scheme(adw_scheme(&saved_scheme));

        // ── Wire: screenshot folder ───────────────────────────────────────
        {
            let fp = file_popover.downgrade();
            screenshot_folder_btn.connect_clicked(move |_| {
                if let Some(f) = fp.upgrade() { f.popdown(); }
                if let Some(pic) = dirs::picture_dir() {
                    let ss = pic.join("Screenshots");
                    let dir = if ss.exists() { ss.join("Aurora Media Player") }
                              else { pic.join("Aurora Media Player") };
                    std::fs::create_dir_all(&dir).ok();
                    std::process::Command::new("xdg-open")
                        .arg(dir.to_string_lossy().as_ref())
                        .spawn().ok();
                }
            });
        }

        // ── Pointer cursor on header buttons ──────────────────────────────
        for w in [
            file_btn.upcast_ref::<gtk::Widget>(),
            playlist_btn.upcast_ref(),
            settings_btn.upcast_ref(),
            open_file_btn.upcast_ref(),
            open_url_btn.upcast_ref(),
            open_sub_btn.upcast_ref(),
            recent_row_btn.upcast_ref(),
            screenshot_folder_btn.upcast_ref(),
        ] {
            w.set_cursor_from_name(Some("pointer"));
        }

        // ── Wire: settings ────────────────────────────────────────────────
        settings_btn.connect_clicked({
            let cb = on_ui_mode_change.clone();
            move |btn| {
                let Some(parent) = btn.root().and_downcast::<gtk::Window>() else { return };
                show_settings_dialog(&parent, cb.clone());
            }
        });

        // ── Wire: open file ───────────────────────────────────────────────
        {
            let on_open_file = Rc::new(on_open_file);
            let fp = file_popover.downgrade();
            let btn_w = file_btn.downgrade();
            open_file_btn.connect_clicked(move |_| {
                if let Some(f) = fp.upgrade() { f.popdown(); }
                let on_open_file = on_open_file.clone();
                let media_filter = gtk::FileFilter::new();
                media_filter.set_name(Some("Media files"));
                for ext in [
                    "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "ts",
                    "mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wma",
                    "m3u", "m3u8",
                ] { media_filter.add_suffix(ext); }
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
                let playlist_filter = gtk::FileFilter::new();
                playlist_filter.set_name(Some("Playlist files"));
                for ext in ["m3u", "m3u8"] { playlist_filter.add_suffix(ext); }
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&media_filter);
                filters.append(&video_filter);
                filters.append(&audio_filter);
                filters.append(&playlist_filter);
                let dialog = gtk::FileDialog::builder()
                    .title("Open Media File").modal(true).filters(&filters).build();
                let parent = btn_w.upgrade().and_then(|b| b.root()).and_downcast::<gtk::Window>();
                dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() { on_open_file(path); }
                    }
                });
            });
        }

        // ── Wire: open URL ────────────────────────────────────────────────
        {
            let on_url_playlist = Rc::new(on_url_playlist);
            let fp = file_popover.downgrade();
            let btn_w = file_btn.downgrade();
            open_url_btn.connect_clicked(move |_| {
                if let Some(f) = fp.upgrade() { f.popdown(); }
                let Some(parent) = btn_w.upgrade()
                    .and_then(|b| b.root())
                    .and_downcast::<gtk::Window>() else { return };
                show_url_playlist_dialog(&parent, on_url_playlist.clone());
            });
        }

        // ── Wire: subtitle ────────────────────────────────────────────────
        {
            let on_open_subtitle = Rc::new(on_open_subtitle);
            let fp = file_popover.downgrade();
            let btn_w = file_btn.downgrade();
            open_sub_btn.connect_clicked(move |_| {
                if let Some(f) = fp.upgrade() { f.popdown(); }
                let on_open_subtitle = on_open_subtitle.clone();
                let sub_filter = gtk::FileFilter::new();
                sub_filter.set_name(Some("Subtitle files"));
                for ext in ["srt", "ass", "ssa", "sub", "vtt", "sup"] {
                    sub_filter.add_suffix(ext);
                }
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&sub_filter);
                let dialog = gtk::FileDialog::builder()
                    .title("Open Subtitle File").modal(true).filters(&filters).build();
                let parent = btn_w.upgrade().and_then(|b| b.root()).and_downcast::<gtk::Window>();
                dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() { on_open_subtitle(path); }
                    }
                });
            });
        }

        // ── Recent files ──────────────────────────────────────────────────
        let on_open_recent = Rc::new(on_open_recent);

        let populate_recent: Rc<dyn Fn()> = {
            let rbox = recent_box.downgrade();
            let fp   = file_popover.downgrade();
            let rs   = recent_sub.downgrade();
            let on_open_recent = on_open_recent.clone();
            Rc::new(move || {
                let Some(rbox) = rbox.upgrade() else { return };
                while let Some(child) = rbox.first_child() { rbox.remove(&child); }
                let entries = load_recent();
                if entries.is_empty() {
                    let lbl = gtk::Label::builder()
                        .label("No recent files")
                        .margin_start(12).margin_end(12)
                        .margin_top(6).margin_bottom(6)
                        .css_classes(["dim-label"])
                        .build();
                    rbox.append(&lbl);
                } else {
                    for entry in entries {
                        // Outer row — scopes the hover state for the remove button.
                        let outer = gtk::Box::builder()
                            .orientation(gtk::Orientation::Horizontal)
                            .spacing(0)
                            .css_classes(["recent-row"])
                            .build();

                        let btn = Button::new();
                        btn.add_css_class("flat");
                        btn.add_css_class("file-menu-item");
                        btn.set_hexpand(true);
                        let row = gtk::Box::builder()
                            .orientation(gtk::Orientation::Horizontal)
                            .spacing(8)
                            .margin_start(8).margin_end(8)
                            .margin_top(2).margin_bottom(2)
                            .build();
                        row.append(&gtk::Image::from_icon_name("document-symbolic"));
                        let lbl = gtk::Label::builder()
                            .label(&entry.title)
                            .halign(gtk::Align::Start)
                            .hexpand(true)
                            .ellipsize(gtk4::pango::EllipsizeMode::End)
                            .build();
                        row.append(&lbl);
                        btn.set_child(Some(&row));
                        btn.set_cursor_from_name(Some("pointer"));
                        let path = std::path::PathBuf::from(&entry.path);
                        let on_open_c = on_open_recent.clone();
                        let fp_w = fp.clone();
                        let rs_w = rs.clone();
                        btn.connect_clicked(move |_| {
                            if let Some(r) = rs_w.upgrade() { r.popdown(); }
                            if let Some(f) = fp_w.upgrade() { f.popdown(); }
                            on_open_c(path.clone());
                        });

                        // Remove button — revealed on hover via CSS.
                        let remove_btn = Button::from_icon_name("window-close-symbolic");
                        remove_btn.add_css_class("flat");
                        remove_btn.add_css_class("circular");
                        remove_btn.add_css_class("recent-remove-btn");
                        remove_btn.set_valign(gtk::Align::Center);
                        remove_btn.set_cursor_from_name(Some("pointer"));
                        remove_btn.set_tooltip_text(Some("Remove from recents"));
                        let entry_path = entry.path.clone();
                        let rbox_w = rbox.clone(); // gtk::Box — already upgraded in this closure
                        let outer_w: glib::WeakRef<gtk::Box> = outer.downgrade();
                        remove_btn.connect_clicked(move |_| {
                            let mut entries = load_recent();
                            entries.retain(|e| e.path != entry_path);
                            save_recent(&entries);
                            if let Some(w) = outer_w.upgrade() { rbox_w.remove(&w); }
                            if rbox_w.first_child().is_none() {
                                let lbl = gtk::Label::builder()
                                    .label("No recent files")
                                    .margin_start(12).margin_end(12)
                                    .margin_top(6).margin_bottom(6)
                                    .css_classes(["dim-label"])
                                    .build();
                                rbox_w.append(&lbl);
                            }
                        });

                        outer.append(&btn);
                        outer.append(&remove_btn);
                        rbox.append(&outer);
                    }
                }
            })
        };
        populate_recent();

        // ── push_recent_fn ────────────────────────────────────────────────
        let populate_recent_c = populate_recent.clone();
        let push_recent_fn: Rc<dyn Fn(&std::path::Path, &str)> = Rc::new(
            move |path: &std::path::Path, title: &str| {
                let mut entries = load_recent();
                let path_str = path.to_string_lossy().to_string();
                entries.retain(|e| e.path != path_str);
                entries.insert(0, RecentEntry { path: path_str, title: title.to_string() });
                entries.truncate(10);
                save_recent(&entries);
                populate_recent_c();
            },
        );

        Self { header, playlist_btn, push_recent_fn, window_title,
               open_file_btn, open_url_btn, open_sub_btn, settings_btn }
    }

    pub fn widget(&self) -> &HeaderBar {
        &self.header
    }

    /// Update the header title/subtitle with the currently playing track.
    /// Pass `None` for `title` when idle to reset to "Aurora".
    pub fn set_now_playing(&self, title: Option<&str>, artist: &str) {
        match title {
            None => {
                self.window_title.set_title("Aurora Media Player");
                self.window_title.set_subtitle("");
            }
            Some(raw) => {
                let is_url_noise = raw.starts_with("http://")
                    || raw.starts_with("https://")
                    || (raw.contains('?') && raw.contains('=') && !raw.contains(' '));
                let display = if is_url_noise || raw.is_empty() { "Loading…" } else { raw };
                self.window_title.set_title(display);
                self.window_title.set_subtitle(artist);
            }
        }
    }
}

// ── Saved URL playlists persistence ──────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SavedPlaylists {
    playlists: Vec<SavedPlaylist>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct SavedPlaylist {
    name: String,
    urls: Vec<String>,
}

fn playlists_path() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("aurora-media").join("url-playlists.json"))
}

fn load_saved_playlists() -> SavedPlaylists {
    let Some(path) = playlists_path() else { return Default::default() };
    let Ok(data) = std::fs::read_to_string(path) else { return Default::default() };
    serde_json::from_str(&data).unwrap_or_default()
}

fn write_saved_playlists(saved: &SavedPlaylists) {
    let Some(path) = playlists_path() else { return };
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).ok(); }
    if let Ok(json) = serde_json::to_string_pretty(saved) {
        std::fs::write(path, json).ok();
    }
}

// ── App settings persistence ──────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct AppSettings {
    /// "system" | "light" | "dark"
    pub color_scheme: Option<String>,
    #[serde(default = "default_volume")]
    pub volume: f64,
    #[serde(default)]
    pub muted: bool,
    /// "floating" (overlay, auto-hide) | "fixed" (always-visible bottom bar)
    pub ui_mode: Option<String>,
}

fn default_volume() -> f64 { 100.0 }

fn settings_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("aurora-media").join("settings.json"))
}

pub fn load_app_settings() -> AppSettings {
    let Some(path) = settings_path() else { return Default::default() };
    let Ok(data) = std::fs::read_to_string(path) else { return Default::default() };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save_app_settings(s: &AppSettings) {
    let Some(path) = settings_path() else { return };
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).ok(); }
    if let Ok(json) = serde_json::to_string_pretty(s) {
        std::fs::write(path, json).ok();
    }
}

fn adw_scheme(key: &str) -> adw::ColorScheme {
    match key {
        "light" => adw::ColorScheme::ForceLight,
        "dark"  => adw::ColorScheme::ForceDark,
        _       => adw::ColorScheme::Default,
    }
}

// ── Settings dialog ───────────────────────────────────────────────────────────

fn show_settings_dialog(parent: &gtk::Window, on_ui_mode_change: Rc<dyn Fn(&str)>) {
    let dialog = adw::Window::builder()
        .title("Settings")
        .transient_for(parent)
        .modal(true)
        .default_width(520)
        .default_height(520)
        .build();

    let header = adw::HeaderBar::new();

    // ── Appearance section ────────────────────────────────────────────────
    let appearance_lbl = gtk::Label::builder()
        .label("Appearance")
        .halign(gtk::Align::Start)
        .css_classes(["heading"])
        .margin_top(18)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();

    let saved_scheme = load_app_settings()
        .color_scheme
        .unwrap_or_else(|| "system".into());

    let theme_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .build();

    let theme_row = adw::ActionRow::builder()
        .title("Theme")
        .build();

    let btn_system = gtk::ToggleButton::builder()
        .label("System")
        .active(saved_scheme == "system")
        .valign(gtk::Align::Center)
        .build();
    let btn_light = gtk::ToggleButton::builder()
        .label("Light")
        .active(saved_scheme == "light")
        .group(&btn_system)
        .valign(gtk::Align::Center)
        .build();
    let btn_dark = gtk::ToggleButton::builder()
        .label("Dark")
        .active(saved_scheme == "dark")
        .group(&btn_system)
        .valign(gtk::Align::Center)
        .build();

    let theme_btns = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["linked"])
        .valign(gtk::Align::Center)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    theme_btns.append(&btn_system);
    theme_btns.append(&btn_light);
    theme_btns.append(&btn_dark);

    theme_row.add_suffix(&theme_btns);
    theme_list.append(&theme_row);

    // ── Keyboard shortcuts section ────────────────────────────────────────
    let shortcuts_lbl = gtk::Label::builder()
        .label("Keyboard Shortcuts")
        .halign(gtk::Align::Start)
        .css_classes(["heading"])
        .margin_top(24)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Helper: build a row with a ShortcutLabel suffix
    let mk_sc = |title: &str, accel: &str| -> adw::ActionRow {
        let row = adw::ActionRow::builder().title(title).build();
        let label = gtk::ShortcutLabel::builder()
            .accelerator(accel)
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&label);
        row
    };

    // ── Playback ──────────────────────────────────────────────────────────
    let playback_lbl = gtk::Label::builder()
        .label("Playback")
        .halign(gtk::Align::Start)
        .css_classes(["caption", "dim-label"])
        .margin_top(6)
        .margin_bottom(2)
        .margin_start(16)
        .build();
    let playback_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .build();
    playback_list.append(&mk_sc("Play / Pause",    "space"));
    playback_list.append(&mk_sc("Next track",      "n"));
    playback_list.append(&mk_sc("Previous track",  "b"));
    playback_list.append(&mk_sc("Mute",            "m"));
    playback_list.append(&mk_sc("Screenshot",      "s"));

    // ── Seek & Volume ─────────────────────────────────────────────────────
    let seek_lbl = gtk::Label::builder()
        .label("Seek & Volume")
        .halign(gtk::Align::Start)
        .css_classes(["caption", "dim-label"])
        .margin_top(12)
        .margin_bottom(2)
        .margin_start(16)
        .build();
    let seek_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .build();
    seek_list.append(&mk_sc("Seek −5 s",       "Left"));
    seek_list.append(&mk_sc("Seek +5 s",       "Right"));
    seek_list.append(&mk_sc("Seek −30 s",      "<Shift>Left"));
    seek_list.append(&mk_sc("Seek +30 s",      "<Shift>Right"));
    seek_list.append(&mk_sc("Volume up",       "Up"));
    seek_list.append(&mk_sc("Volume down",     "Down"));

    // ── Speed & Video ─────────────────────────────────────────────────────
    let video_lbl = gtk::Label::builder()
        .label("Speed & Video")
        .halign(gtk::Align::Start)
        .css_classes(["caption", "dim-label"])
        .margin_top(12)
        .margin_bottom(2)
        .margin_start(16)
        .build();
    let video_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .build();
    video_list.append(&mk_sc("Speed up",          "bracketright"));
    video_list.append(&mk_sc("Speed down",        "bracketleft"));
    video_list.append(&mk_sc("Reset speed",       "BackSpace"));
    video_list.append(&mk_sc("Fullscreen",        "f"));
    video_list.append(&mk_sc("Exit fullscreen",   "Escape"));

    // ── App ───────────────────────────────────────────────────────────────
    let app_lbl = gtk::Label::builder()
        .label("App")
        .halign(gtk::Align::Start)
        .css_classes(["caption", "dim-label"])
        .margin_top(12)
        .margin_bottom(2)
        .margin_start(16)
        .build();
    let app_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .build();
    app_list.append(&mk_sc("Open file",        "<Primary>o"));
    app_list.append(&mk_sc("Open URL",         "<Primary>u"));
    app_list.append(&mk_sc("Load subtitle",    "<Primary>t"));
    app_list.append(&mk_sc("Toggle playlist",  "<Primary>p"));
    app_list.append(&mk_sc("Settings",         "<Primary>comma"));

    // ── Footer ────────────────────────────────────────────────────────────
    let footer = gtk::Label::builder()
        .use_markup(true)
        .wrap(true)
        .max_width_chars(36)
        .justify(gtk::Justification::Center)
        .css_classes(["caption", "dim-label"])
        .halign(gtk::Align::Center)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(12)
        .margin_end(12)
        .build();
    footer.set_markup(
        "If you like Aurora Media Player, consider\n<a href=\"https://github.com/daniacosta-dev/aurora-media-player\">⭐ starring it on GitHub</a>"
    );

    // ── Control bar section ─────────────────────────────────────────────
    let layout_lbl = gtk::Label::builder()
        .label("Control bar")
        .halign(gtk::Align::Start)
        .css_classes(["heading"])
        .margin_top(24)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();

    let saved_ui_mode = load_app_settings()
        .ui_mode
        .unwrap_or_else(|| "floating".into());

    let layout_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .build();

    let layout_row = adw::ActionRow::builder()
        .title("Control bar style")
        .build();

    let btn_floating = gtk::ToggleButton::builder()
        .label("Floating")
        .active(saved_ui_mode == "floating")
        .valign(gtk::Align::Center)
        .build();
    let btn_fixed = gtk::ToggleButton::builder()
        .label("Fixed")
        .active(saved_ui_mode == "fixed")
        .group(&btn_floating)
        .valign(gtk::Align::Center)
        .build();

    let layout_btns = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["linked"])
        .valign(gtk::Align::Center)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    layout_btns.append(&btn_floating);
    layout_btns.append(&btn_fixed);

    layout_row.add_suffix(&layout_btns);
    layout_list.append(&layout_row);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_bottom(32)
        .build();
    content.append(&appearance_lbl);
    content.append(&theme_list);
    content.append(&layout_lbl);
    content.append(&layout_list);
    content.append(&shortcuts_lbl);
    content.append(&playback_lbl);
    content.append(&playback_list);
    content.append(&seek_lbl);
    content.append(&seek_list);
    content.append(&video_lbl);
    content.append(&video_list);
    content.append(&app_lbl);
    content.append(&app_list);
    content.append(&footer);

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .margin_bottom(12)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&scroll));
    dialog.set_content(Some(&toolbar_view));

    // Wire theme buttons → StyleManager + persistence
    btn_system.connect_toggled(|btn| {
        if btn.is_active() {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);
            let mut s = load_app_settings(); s.color_scheme = Some("system".into()); save_app_settings(&s);
        }
    });
    btn_light.connect_toggled(|btn| {
        if btn.is_active() {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
            let mut s = load_app_settings(); s.color_scheme = Some("light".into()); save_app_settings(&s);
        }
    });
    btn_dark.connect_toggled(|btn| {
        if btn.is_active() {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
            let mut s = load_app_settings(); s.color_scheme = Some("dark".into()); save_app_settings(&s);
        }
    });

    btn_floating.connect_toggled({
        let cb = on_ui_mode_change.clone();
        move |btn| {
            if btn.is_active() {
                let mut s = load_app_settings(); s.ui_mode = Some("floating".into()); save_app_settings(&s);
                cb("floating");
            }
        }
    });
    btn_fixed.connect_toggled({
        let cb = on_ui_mode_change.clone();
        move |btn| {
            if btn.is_active() {
                let mut s = load_app_settings(); s.ui_mode = Some("fixed".into()); save_app_settings(&s);
                cb("fixed");
            }
        }
    });

    dialog.present();
}

// ── URL playlist dialog ───────────────────────────────────────────────────────

/// True if the URL path (before `?`) ends with `.m3u` or `.m3u8`.
fn looks_like_m3u_url(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url).to_lowercase();
    path.ends_with(".m3u") || path.ends_with(".m3u8")
}

/// Derive a hostname-based fallback title from a URL.
fn title_for_url(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .and_then(|host| host.split('?').next())
        .unwrap_or("URL")
        .to_string()
}

/// Fetch an M3U URL with curl and return `(title, stream_url)` pairs.
/// Titles come from `#EXTINF` lines; falls back to hostname when absent.
/// Called from a background thread — must not touch GTK objects.
fn fetch_and_parse_m3u(url: &str) -> Vec<(String, String)> {
    let output = std::process::Command::new("curl")
        .args(["-s", "-L", "--max-time", "15", url])
        .output()
        .ok();
    let Some(output) = output else { return vec![] };
    let Ok(content) = String::from_utf8(output.stdout) else { return vec![] };

    let mut result = Vec::new();
    let mut pending_title: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            // #EXTINF:duration[,...],Title
            if let Some((_, title)) = rest.split_once(',') {
                let title = title.trim();
                if !title.is_empty() {
                    pending_title = Some(title.to_string());
                }
            }
        } else if line.starts_with('#') {
            continue; // skip other directives
        } else if line.starts_with("http://") || line.starts_with("https://") {
            let title = pending_title.take()
                .unwrap_or_else(|| title_for_url(line));
            result.push((title, line.to_string()));
        }
    }
    result
}

/// Expand any M3U URLs in `urls` into `(title, stream_url)` pairs.
/// Non-M3U URLs are kept as-is with a hostname-based title.
/// Called from a background thread.
fn expand_m3u_urls(urls: Vec<String>) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for url in urls {
        if looks_like_m3u_url(&url) {
            let fetched = fetch_and_parse_m3u(&url);
            if fetched.is_empty() {
                result.push((title_for_url(&url), url)); // fallback: keep original
            } else {
                result.extend(fetched);
            }
        } else {
            result.push((title_for_url(&url), url));
        }
    }
    result
}

/// Returns `Some(error_message)` if the URL is not a valid http/https URL,
/// or `None` if it looks fine.  Only checks syntax — not reachability.
fn validate_url(url: &str) -> Option<String> {
    let url = url.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Some("Must start with http:// or https://".into());
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or("");
    let host = rest.split('/').next().unwrap_or("").split('?').next().unwrap_or("");
    if host.is_empty() {
        return Some("Missing hostname".into());
    }
    None
}

fn show_url_playlist_dialog(
    parent: &gtk::Window,
    on_load: Rc<impl Fn(Vec<(String, String)>) + 'static>,
) {
    let dialog = adw::Window::builder()
        .title("URL Playlist")
        .transient_for(parent)
        .modal(true)
        .default_width(520)
        .default_height(520)
        .build();

    // ── Header ────────────────────────────────────────────────────────────
    let header = adw::HeaderBar::new();
    let play_btn = Button::builder()
        .label("Play")
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    header.pack_end(&play_btn);

    // ── URL entry list ────────────────────────────────────────────────────
    let list_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_top(12)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Add URL sentinel row (always at the bottom of the entry list)
    let add_row = gtk::ListBoxRow::builder().activatable(true).build();
    let add_label = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    add_label.append(&gtk::Image::from_icon_name("list-add-symbolic"));
    add_label.append(&gtk::Label::new(Some("Add URL")));
    add_row.set_child(Some(&add_label));
    list_box.append(&add_row);

    // ── Save-as row ───────────────────────────────────────────────────────
    let save_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    let name_row = adw::EntryRow::builder().title("Save as playlist…").build();
    let save_btn = Button::builder()
        .label("Save")
        .css_classes(["flat"])
        .tooltip_text("Save playlist")
        .sensitive(false)
        .build();
    name_row.add_suffix(&save_btn);
    save_list.append(&name_row);

    // ── Saved playlists list (hidden when empty) ──────────────────────────
    let saved_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_top(0)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .visible(false)
        .build();

    let vbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    vbox.append(&list_box);
    vbox.append(&save_list);
    vbox.append(&saved_list);

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&vbox)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&scroll));
    dialog.set_content(Some(&toolbar_view));

    // ── Entry row tracking ────────────────────────────────────────────────
    let entries: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));

    // Late-bound: loads a saved playlist's URLs into the editor rows.
    let load_into_editor: Rc<RefCell<Option<Box<dyn Fn(Vec<String>, String)>>>> =
        Rc::new(RefCell::new(None));

    // ── Saved-playlist section rebuild (late-bound to break circular ref) ─
    let rebuild_saved: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    {
        let saved_list_c = saved_list.clone();
        let on_load_c = on_load.clone();
        let dialog_c = dialog.clone();
        let rebuild_ref = rebuild_saved.clone();
        let lie_ref = load_into_editor.clone();
        let name_row_rs = name_row.clone();
        *rebuild_saved.borrow_mut() = Some(Box::new(move || {
            while let Some(child) = saved_list_c.first_child() {
                saved_list_c.remove(&child);
            }
            let saved = load_saved_playlists();
            // Hide the playlist currently loaded in the editor to avoid redundancy.
            let editing = name_row_rs.text();
            let editing = editing.trim().to_string();
            let visible: Vec<_> = saved.playlists.iter()
                .filter(|p| p.name != editing)
                .collect();
            saved_list_c.set_visible(!visible.is_empty());
            for playlist in &visible {
                let row = gtk::ListBoxRow::builder().activatable(false).build();
                let hbox = gtk::Box::builder()
                    .orientation(gtk::Orientation::Horizontal)
                    .spacing(8)
                    .margin_top(8)
                    .margin_bottom(8)
                    .margin_start(12)
                    .margin_end(12)
                    .build();
                let label = gtk::Label::builder()
                    .label(&playlist.name)
                    .hexpand(true)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build();
                let edit_btn = Button::builder()
                    .icon_name("document-edit-symbolic")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat", "circular"])
                    .tooltip_text("Edit")
                    .build();
                let load_btn = Button::builder()
                    .icon_name("media-playback-start-symbolic")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat", "circular"])
                    .tooltip_text("Play")
                    .build();
                let delete_btn = Button::builder()
                    .icon_name("edit-delete-symbolic")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat", "circular"])
                    .tooltip_text("Delete")
                    .build();
                hbox.append(&label);
                hbox.append(&edit_btn);
                hbox.append(&load_btn);
                hbox.append(&delete_btn);
                row.set_child(Some(&hbox));
                saved_list_c.append(&row);

                let urls_e = playlist.urls.clone();
                let name_e = playlist.name.clone();
                let lie_e = lie_ref.clone();
                edit_btn.connect_clicked(move |_| {
                    if let Some(f) = &*lie_e.borrow() {
                        f(urls_e.clone(), name_e.clone());
                    }
                });

                let urls = playlist.urls.clone();
                let on_load_l = on_load_c.clone();
                let dialog_l = dialog_c.clone();
                load_btn.connect_clicked(move |btn| {
                    // Expand any M3U URLs before loading, same as the editor's Play button.
                    if urls.iter().any(|u| looks_like_m3u_url(u)) {
                        btn.set_sensitive(false);
                        let urls_c = urls.clone();
                        let (tx, rx) = std::sync::mpsc::channel::<Vec<(String, String)>>();
                        std::thread::spawn(move || { tx.send(expand_m3u_urls(urls_c)).ok(); });
                        let on_load_t = on_load_l.clone();
                        let dialog_t = dialog_l.clone();
                        let btn_w = btn.downgrade();
                        glib::timeout_add_local(Duration::from_millis(100), move || {
                            match rx.try_recv() {
                                Ok(expanded) => {
                                    if !expanded.is_empty() { on_load_t(expanded); }
                                    dialog_t.close();
                                    glib::ControlFlow::Break
                                }
                                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                                Err(_) => {
                                    if let Some(b) = btn_w.upgrade() { b.set_sensitive(true); }
                                    glib::ControlFlow::Break
                                }
                            }
                        });
                    } else {
                        let items = urls.iter().map(|u| (title_for_url(u), u.clone())).collect();
                        on_load_l(items);
                        dialog_l.close();
                    }
                });

                let name_d = playlist.name.clone();
                let rebuild_d = rebuild_ref.clone();
                delete_btn.connect_clicked(move |_| {
                    let mut saved = load_saved_playlists();
                    saved.playlists.retain(|p| p.name != name_d);
                    write_saved_playlists(&saved);
                    if let Some(f) = &*rebuild_d.borrow() { f(); }
                });
            }
        }));
    }
    if let Some(f) = &*rebuild_saved.borrow() { f(); }

    // ── add_row sensitivity: disabled when last entry is empty ────────────
    let update_add_row = {
        let entries_c = entries.clone();
        let add_row_c = add_row.clone();
        Rc::new(move || {
            let sensitive = entries_c
                .borrow()
                .last()
                .map(|e| !e.text().trim().is_empty() && !e.has_css_class("error"))
                .unwrap_or(true); // no entries yet → allow adding the first
            add_row_c.set_sensitive(sensitive);
        })
    };

    // ── save_btn sensitivity: name non-empty, at least one valid URL,
    //    and no outstanding invalid (non-empty) entries ─────────────────────
    let update_save_btn = {
        let entries_c = entries.clone();
        let name_row_c = name_row.clone();
        let save_btn_c = save_btn.clone();
        Rc::new(move || {
            let has_name = !name_row_c.text().trim().is_empty();
            let ents = entries_c.borrow();
            let has_valid = ents.iter().any(|e| {
                let t = e.text(); !t.trim().is_empty() && !e.has_css_class("error")
            });
            let has_invalid = ents.iter().any(|e| {
                let t = e.text(); !t.trim().is_empty() && e.has_css_class("error")
            });
            save_btn_c.set_sensitive(has_name && has_valid && !has_invalid);
        })
    };

    // ── Append entry row before the sentinel ─────────────────────────────
    let append_entry = {
        let list_box_c = list_box.clone();
        let entries_c = entries.clone();
        let play_btn_c = play_btn.clone();
        let update_add_c = update_add_row.clone();
        let update_save_c = update_save_btn.clone();

        Rc::new(move || {
            let entry_row = adw::EntryRow::builder()
                .title("URL")
                .show_apply_button(false)
                .build();

            // Warning icon shown when the URL format is invalid.
            let warn_icon = gtk::Image::builder()
                .icon_name("dialog-warning-symbolic")
                .css_classes(["warning"])
                .visible(false)
                .build();
            let remove_btn = Button::builder()
                .icon_name("list-remove-symbolic")
                .valign(gtk::Align::Center)
                .css_classes(["flat", "circular"])
                .tooltip_text("Remove")
                .build();
            entry_row.add_suffix(&warn_icon);
            entry_row.add_suffix(&remove_btn);

            let pos = list_box_c.observe_children().n_items().saturating_sub(1) as i32;
            list_box_c.insert(&entry_row, pos);
            entries_c.borrow_mut().push(entry_row.clone());

            {
                let entries_s = entries_c.clone();
                let play_btn_s = play_btn_c.clone();
                let update_add_s = update_add_c.clone();
                let update_save_s = update_save_c.clone();
                let row_s = entry_row.clone();
                let warn_s = warn_icon.clone();
                entry_row.connect_changed(move |row| {
                    let text = row.text();
                    let text = text.trim();
                    if text.is_empty() {
                        row_s.remove_css_class("error");
                        warn_s.set_visible(false);
                        warn_s.set_tooltip_text(None);
                    } else if let Some(err) = validate_url(text) {
                        row_s.add_css_class("error");
                        warn_s.set_visible(true);
                        warn_s.set_tooltip_text(Some(&err));
                    } else {
                        row_s.remove_css_class("error");
                        warn_s.set_visible(false);
                        warn_s.set_tooltip_text(None);
                    }
                    let has_valid = entries_s.borrow().iter().any(|e| {
                        let t = e.text(); !t.trim().is_empty() && !e.has_css_class("error")
                    });
                    play_btn_s.set_sensitive(has_valid);
                    update_add_s();
                    update_save_s();
                });
            }

            {
                let list_box_r = list_box_c.clone();
                let entries_r = entries_c.clone();
                let play_btn_r = play_btn_c.clone();
                let update_add_r = update_add_c.clone();
                let update_save_r = update_save_c.clone();
                let entry_row_r = entry_row.clone();
                remove_btn.connect_clicked(move |_| {
                    list_box_r.remove(&entry_row_r);
                    entries_r.borrow_mut().retain(|e| e != &entry_row_r);
                    let has_valid = entries_r.borrow().iter().any(|e| {
                        let t = e.text(); !t.trim().is_empty() && !e.has_css_class("error")
                    });
                    play_btn_r.set_sensitive(has_valid);
                    update_add_r();
                    update_save_r();
                });
            }

            update_add_c();
        })
    };

    // ── Wire load_into_editor (needs append_entry, so set here) ──────────
    {
        let entries_l = entries.clone();
        let list_box_l = list_box.clone();
        let append_l = append_entry.clone();
        let name_row_l = name_row.clone();
        let play_btn_l = play_btn.clone();
        let update_add_l = update_add_row.clone();
        let update_save_l = update_save_btn.clone();
        let rebuild_l = rebuild_saved.clone();
        *load_into_editor.borrow_mut() = Some(Box::new(move |urls: Vec<String>, name: String| {
            // Remove all current entry rows from the list.
            {
                let mut ents = entries_l.borrow_mut();
                for entry in ents.drain(..) {
                    list_box_l.remove(&entry);
                }
            }
            // Re-add one row per saved URL, pre-filled.
            // Clone the entry out before calling set_text to avoid a double-borrow
            // (set_text fires connect_changed which also borrows entries_l).
            for url in urls {
                append_l();
                let entry = entries_l.borrow().last().cloned();
                if let Some(entry) = entry {
                    entry.set_text(&url);
                }
            }
            // Pre-fill playlist name so saving will update the existing entry.
            name_row_l.set_text(&name);
            // Refresh sensitivities.
            let has_valid = entries_l.borrow().iter().any(|e| {
                let t = e.text(); !t.trim().is_empty() && !e.has_css_class("error")
            });
            play_btn_l.set_sensitive(has_valid);
            update_add_l();
            update_save_l();
            // Rebuild the saved list so the edited playlist disappears from it.
            if let Some(f) = &*rebuild_l.borrow() { f(); }
        }));
    }

    // Start with one empty entry (add_row will be insensitive initially).
    append_entry();

    // "Add URL" sentinel click → append another entry.
    {
        let append_c = append_entry.clone();
        list_box.connect_row_activated(move |_, row| {
            if row.child().and_downcast_ref::<adw::EntryRow>().is_none() {
                append_c();
            }
        });
    }

    // name_row changes → update save sensitivity + refresh saved list
    // (the saved list hides whichever playlist matches the current name).
    {
        let update_save_c = update_save_btn.clone();
        let rebuild_c2 = rebuild_saved.clone();
        name_row.connect_changed(move |_| {
            update_save_c();
            if let Some(f) = &*rebuild_c2.borrow() { f(); }
        });
    }

    // ── Save button ───────────────────────────────────────────────────────
    {
        let entries_c = entries.clone();
        let name_row_c = name_row.clone();
        let rebuild_c = rebuild_saved.clone();
        save_btn.connect_clicked(move |_| {
            let name = name_row_c.text().trim().to_string();
            let urls: Vec<String> = entries_c
                .borrow()
                .iter()
                .map(|e| e.text().trim().to_string())
                .filter(|u| !u.is_empty())
                .collect();
            if name.is_empty() || urls.is_empty() { return; }
            let mut saved = load_saved_playlists();
            if let Some(existing) = saved.playlists.iter_mut().find(|p| p.name == name) {
                existing.urls = urls;
            } else {
                saved.playlists.push(SavedPlaylist { name, urls });
            }
            write_saved_playlists(&saved);
            name_row_c.set_text("");
            if let Some(f) = &*rebuild_c.borrow() { f(); }
        });
    }

    // ── Play All ──────────────────────────────────────────────────────────
    {
        let entries_c = entries.clone();
        let dialog_c = dialog.clone();
        let play_btn_w = play_btn.downgrade();
        play_btn.connect_clicked(move |btn| {
            let urls: Vec<String> = entries_c
                .borrow()
                .iter()
                .filter(|e| !e.has_css_class("error"))
                .map(|e| e.text().trim().to_string())
                .filter(|u| !u.is_empty())
                .collect();
            if urls.is_empty() { return; }

            // If any URL looks like an M3U playlist, fetch and expand it
            // in a background thread so the UI stays responsive.
            if urls.iter().any(|u| looks_like_m3u_url(u)) {
                btn.set_sensitive(false);
                btn.set_label("Loading…");

                let (tx, rx) = std::sync::mpsc::channel::<Vec<(String, String)>>();
                std::thread::spawn(move || { tx.send(expand_m3u_urls(urls)).ok(); });

                let on_load_c = on_load.clone();
                let dialog_c2 = dialog_c.clone();
                let play_btn_w2 = play_btn_w.clone();
                glib::timeout_add_local(Duration::from_millis(100), move || {
                    match rx.try_recv() {
                        Ok(expanded) => {
                            if !expanded.is_empty() { on_load_c(expanded); }
                            dialog_c2.close();
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => {
                            if let Some(btn) = play_btn_w2.upgrade() {
                                btn.set_sensitive(true);
                                btn.set_label("Play");
                            }
                            glib::ControlFlow::Break
                        }
                    }
                });
            } else {
                let items = urls.into_iter().map(|u| (title_for_url(&u), u)).collect();
                on_load(items);
                dialog_c.close();
            }
        });
    }

    dialog.present();
}
