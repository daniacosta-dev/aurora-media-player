use adw::{NavigationPage, ToolbarView, HeaderBar};
use gtk4::{self as gtk, ListBox, ListBoxRow, Label, ScrolledWindow, SelectionMode, Box, Orientation};
use gtk4::prelude::*;
use std::path::PathBuf;

use crate::state::SharedState;
use crate::player::PlayerCommand;

pub struct PlaylistPanel {
    page: NavigationPage,
    list: ListBox,
}

impl PlaylistPanel {
    pub fn new(state: SharedState) -> Self {
        let list = ListBox::builder()
            .selection_mode(SelectionMode::Single)
            .css_classes(vec!["boxed-list"])
            .build();

        let scroll = ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .build();

        let toolbar = ToolbarView::builder()
            .content(&scroll)
            .build();

        let header = HeaderBar::builder()
            .title_widget(&gtk::Label::new(Some("Playlist")))
            .show_back_button(false)
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .build();
        toolbar.add_top_bar(&header);

        let page = NavigationPage::builder()
            .child(&toolbar)
            .title("Playlist")
            .build();

        // ── Signal: activate row → play that file ─────────────────────────
        {
            let state_c = state.clone();
            list.connect_row_activated(move |_, row| {
                let idx = row.index() as usize;
                let path = {
                    let mut s = state_c.borrow_mut();
                    s.current_idx = Some(idx);
                    s.playlist.get(idx).cloned()
                };
                if let Some(path) = path {
                    if let Some(p) = state_c.borrow().player.as_ref() {
                        p.execute(PlayerCommand::Open(path)).ok();
                    }
                }
            });
        }

        Self { page, list }
    }

    pub fn widget(&self) -> &NavigationPage {
        &self.page
    }

    /// Append one file to the visible list.
    pub fn add_item(&self, title: &str, _path: &PathBuf) {
        let row = ListBoxRow::new();
        let label = Label::builder()
            .label(title)
            .halign(gtk::Align::Start)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        let inner = Box::new(Orientation::Horizontal, 0);
        inner.append(&label);
        row.set_child(Some(&inner));
        self.list.append(&row);
    }

    /// Remove all rows from the list.
    pub fn clear(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
    }

    /// Highlight the row at `idx` without emitting `row-activated`.
    pub fn select_row(&self, idx: usize) {
        if let Some(row) = self.list.row_at_index(idx as i32) {
            self.list.select_row(Some(&row));
        }
    }
}
