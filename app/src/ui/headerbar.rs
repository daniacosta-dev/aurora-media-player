use std::path::PathBuf;
use std::rc::Rc;
use std::cell::RefCell;
use std::time::Duration;

use adw::{self, HeaderBar};
use adw::prelude::*;
use gtk4::{self as gtk, Button, ToggleButton, Popover};
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
    /// Exposed so the window can bind it to the OverlaySplitView's
    /// `show-sidebar` property.
    pub playlist_btn: ToggleButton,
    pub push_recent_fn: Rc<dyn Fn(&std::path::Path, &str)>,
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
    ) -> Self {
        let header = HeaderBar::new();

        // ── Open file button ──────────────────────────────────────────────
        let open_btn = Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Open file")
            .build();
        header.pack_start(&open_btn);

        // ── Subtitle file button ──────────────────────────────────────────
        let sub_btn = Button::builder()
            .icon_name("media-view-subtitles-symbolic")
            .tooltip_text("Load subtitle file")
            .build();
        header.pack_start(&sub_btn);

        // ── Recent files button ───────────────────────────────────────────
        let recent_btn = Button::builder()
            .icon_name("document-open-recent-symbolic")
            .tooltip_text("Recent files")
            .build();
        header.pack_start(&recent_btn);

        let recent_popover = Popover::new();
        recent_popover.set_parent(&recent_btn);
        {
            let rp = recent_popover.clone();
            recent_btn.connect_clicked(move |_| { rp.popup(); });
        }

        // ── Open URL / URL playlist button ────────────────────────────────
        let url_btn = Button::builder()
            .icon_name("insert-link-symbolic")
            .tooltip_text("Open URL or URL playlist")
            .build();
        header.pack_start(&url_btn);

        // ── Playlist toggle ───────────────────────────────────────────────
        let playlist_btn = ToggleButton::builder()
            .icon_name("view-list-symbolic")
            .tooltip_text("Toggle playlist")
            .build();
        header.pack_end(&playlist_btn);

        // ── Settings button ───────────────────────────────────────────────
        let settings_btn = Button::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Settings")
            .build();
        header.pack_end(&settings_btn);

        // Apply persisted color scheme immediately on construction.
        let saved_scheme = load_app_settings()
            .color_scheme
            .unwrap_or_else(|| "system".into());
        adw::StyleManager::default().set_color_scheme(adw_scheme(&saved_scheme));

        // ── Wire: settings button → settings dialog ───────────────────────
        settings_btn.connect_clicked(|btn| {
            let Some(parent) = btn.root().and_downcast::<gtk::Window>() else { return };
            show_settings_dialog(&parent);
        });

        // ── Wire: open button → GTK FileDialog (portal-backed) ────────────
        {
            let on_open_file = Rc::new(on_open_file);
            open_btn.connect_clicked(move |btn| {
                let on_open_file = on_open_file.clone();
                let media_filter = gtk::FileFilter::new();
                media_filter.set_name(Some("Media files"));
                for ext in [
                    "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "ts",
                    "mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wma",
                    "m3u", "m3u8",
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

                let playlist_filter = gtk::FileFilter::new();
                playlist_filter.set_name(Some("Playlist files"));
                for ext in ["m3u", "m3u8"] {
                    playlist_filter.add_suffix(ext);
                }

                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&media_filter);
                filters.append(&video_filter);
                filters.append(&audio_filter);
                filters.append(&playlist_filter);

                let dialog = gtk::FileDialog::builder()
                    .title("Open Media File")
                    .modal(true)
                    .filters(&filters)
                    .build();

                let parent = btn.root().and_downcast::<gtk::Window>();
                dialog.open(
                    parent.as_ref(),
                    None::<&gio::Cancellable>,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                on_open_file(path);
                            }
                        }
                    },
                );
            });
        }

        // ── Wire: subtitle button → file dialog ──────────────────────────
        {
            let on_open_subtitle = Rc::new(on_open_subtitle);
            sub_btn.connect_clicked(move |btn| {
                let on_open_subtitle = on_open_subtitle.clone();
                let sub_filter = gtk::FileFilter::new();
                sub_filter.set_name(Some("Subtitle files"));
                for ext in ["srt", "ass", "ssa", "sub", "vtt", "sup"] {
                    sub_filter.add_suffix(ext);
                }
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&sub_filter);
                let dialog = gtk::FileDialog::builder()
                    .title("Open Subtitle File")
                    .modal(true)
                    .filters(&filters)
                    .build();
                let parent = btn.root().and_downcast::<gtk::Window>();
                dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            on_open_subtitle(path);
                        }
                    }
                });
            });
        }

        // ── Wire: recent popover → rebuild on show ────────────────────────
        let on_open_recent = Rc::new(on_open_recent);
        {
            let on_open_c = on_open_recent.clone();
            recent_popover.connect_show(move |popover| {
                let entries = load_recent();
                // Clear old content
                popover.set_child(None::<&gtk::Widget>);
                if entries.is_empty() {
                    let lbl = gtk::Label::builder()
                        .label("No recent files")
                        .css_classes(vec!["dim-label"])
                        .margin_top(8).margin_bottom(8)
                        .margin_start(12).margin_end(12)
                        .build();
                    popover.set_child(Some(&lbl));
                    return;
                }
                let vbox = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(2)
                    .margin_top(4).margin_bottom(4)
                    .margin_start(4).margin_end(4)
                    .build();
                for entry in &entries {
                    let btn = Button::builder()
                        .label(&entry.title)
                        .css_classes(vec!["flat"])
                        .halign(gtk::Align::Fill)
                        .build();
                    let path = std::path::PathBuf::from(&entry.path);
                    let on_open = on_open_c.clone();
                    let popover_weak = popover.downgrade();
                    btn.connect_clicked(move |_| {
                        on_open(path.clone());
                        if let Some(p) = popover_weak.upgrade() {
                            p.popdown();
                        }
                    });
                    vbox.append(&btn);
                }
                popover.set_child(Some(&vbox));
            });
        }

        // ── push_recent_fn closure ────────────────────────────────────────
        let push_recent_fn: Rc<dyn Fn(&std::path::Path, &str)> = Rc::new(move |path: &std::path::Path, title: &str| {
            let mut entries = load_recent();
            let path_str = path.to_string_lossy().to_string();
            entries.retain(|e| e.path != path_str);
            entries.insert(0, RecentEntry { path: path_str, title: title.to_string() });
            entries.truncate(10);
            save_recent(&entries);
        });

        // ── Wire: URL button → URL playlist dialog ────────────────────────
        {
            let on_url_playlist = Rc::new(on_url_playlist);
            url_btn.connect_clicked(move |btn| {
                let Some(parent) = btn.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                show_url_playlist_dialog(&parent, on_url_playlist.clone());
            });
        }

        Self { header, playlist_btn, push_recent_fn }
    }

    pub fn widget(&self) -> &HeaderBar {
        &self.header
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

fn show_settings_dialog(parent: &gtk::Window) {
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

    let theme_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .build();

    let saved_scheme = load_app_settings()
        .color_scheme
        .unwrap_or_else(|| "system".into());

    let mk_row = |title: &str, key: &str| -> (adw::ActionRow, gtk::CheckButton) {
        let row = adw::ActionRow::builder()
            .title(title)
            .activatable(true)
            .build();
        let check = gtk::CheckButton::builder()
            .active(saved_scheme == key)
            .valign(gtk::Align::Center)
            .build();
        row.add_suffix(&check);
        row.set_activatable_widget(Some(&check));
        (row, check)
    };

    let (row_system, check_system) = mk_row("Follow system", "system");
    let (row_light,  check_light)  = mk_row("Light", "light");
    let (row_dark,   check_dark)   = mk_row("Dark",  "dark");

    check_light.set_group(Some(&check_system));
    check_dark.set_group(Some(&check_system));

    theme_list.append(&row_system);
    theme_list.append(&row_light);
    theme_list.append(&row_dark);

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

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&appearance_lbl);
    content.append(&theme_list);
    content.append(&footer);

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&scroll));
    dialog.set_content(Some(&toolbar_view));

    // Wire radio buttons → StyleManager + persistence
    check_system.connect_toggled(|btn| {
        if btn.is_active() {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);
            let mut s = load_app_settings();
            s.color_scheme = Some("system".into());
            save_app_settings(&s);
        }
    });
    check_light.connect_toggled(|btn| {
        if btn.is_active() {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
            let mut s = load_app_settings();
            s.color_scheme = Some("light".into());
            save_app_settings(&s);
        }
    });
    check_dark.connect_toggled(|btn| {
        if btn.is_active() {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
            let mut s = load_app_settings();
            s.color_scheme = Some("dark".into());
            save_app_settings(&s);
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
        .label("Play All")
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
                load_btn.connect_clicked(move |_| {
                    let items = urls.iter()
                        .map(|u| (title_for_url(u), u.clone()))
                        .collect();
                    on_load_l(items);
                    dialog_l.close();
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
                                btn.set_label("Play All");
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
