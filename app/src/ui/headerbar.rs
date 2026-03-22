use std::rc::Rc;
use std::cell::RefCell;

use adw::{self, HeaderBar};
use adw::prelude::*;
use gtk4::{self as gtk, Button, ToggleButton};
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
    /// `on_url_playlist` is called when the user confirms a URL playlist.
    /// The callback receives the list of URLs and is responsible for loading
    /// them into the player playlist (wired in window.rs).
    pub fn new(
        state: SharedState,
        on_url_playlist: impl Fn(Vec<String>) + 'static,
    ) -> Self {
        let header = HeaderBar::new();

        // ── Open file button ──────────────────────────────────────────────
        let open_btn = Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Open file")
            .build();
        header.pack_start(&open_btn);

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
            let state_c = state.clone();
            open_btn.connect_clicked(move |btn| {
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

                let parent = btn.root().and_downcast::<gtk::Window>();
                let state_inner = state_c.clone();
                dialog.open(
                    parent.as_ref(),
                    None::<&gio::Cancellable>,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                state_inner.borrow_mut().pending_seek = None;
                                if let Some(p) = state_inner.borrow().player.as_ref() {
                                    if let Err(e) = p.execute(PlayerCommand::Open(path)) {
                                        log::error!("open file: {e}");
                                    }
                                }
                            }
                        }
                    },
                );
            });
        }

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

        Self { header, playlist_btn }
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
struct AppSettings {
    /// "system" | "light" | "dark"
    color_scheme: Option<String>,
}

fn settings_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("aurora-media").join("settings.json"))
}

fn load_app_settings() -> AppSettings {
    let Some(path) = settings_path() else { return Default::default() };
    let Ok(data) = std::fs::read_to_string(path) else { return Default::default() };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_app_settings(s: &AppSettings) {
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
            save_app_settings(&AppSettings { color_scheme: Some("system".into()) });
        }
    });
    check_light.connect_toggled(|btn| {
        if btn.is_active() {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
            save_app_settings(&AppSettings { color_scheme: Some("light".into()) });
        }
    });
    check_dark.connect_toggled(|btn| {
        if btn.is_active() {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
            save_app_settings(&AppSettings { color_scheme: Some("dark".into()) });
        }
    });

    dialog.present();
}

// ── URL playlist dialog ───────────────────────────────────────────────────────

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
    on_load: Rc<impl Fn(Vec<String>) + 'static>,
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
                    on_load_l(urls.clone());
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
        play_btn.connect_clicked(move |_| {
            let urls: Vec<String> = entries_c
                .borrow()
                .iter()
                .filter(|e| !e.has_css_class("error"))
                .map(|e| e.text().trim().to_string())
                .filter(|u| !u.is_empty())
                .collect();
            if !urls.is_empty() {
                on_load(urls);
            }
            dialog_c.close();
        });
    }

    dialog.present();
}
