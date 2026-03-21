use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use gtk4::{self as gtk, GLArea, Overlay, Label, Stack, Box, Orientation, Image};
use gtk4::prelude::*;
use glib;
use libc;

use crate::state::SharedState;

const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

fn current_fbo() -> i32 {
    // Resolve glGetIntegerv at runtime via dlsym so we use whatever GL
    // implementation epoxy/GTK has already loaded (GLX or EGL).
    type GlGetIntegervFn = unsafe extern "C" fn(pname: u32, params: *mut i32);
    let sym = unsafe {
        libc::dlsym(libc::RTLD_DEFAULT, b"glGetIntegerv\0".as_ptr() as *const libc::c_char)
    };
    if sym.is_null() {
        return 0;
    }
    let gl_get_integerv: GlGetIntegervFn = unsafe { std::mem::transmute(sym) };
    let mut fbo: i32 = 0;
    unsafe { gl_get_integerv(GL_DRAW_FRAMEBUFFER_BINDING, &mut fbo) };
    fbo
}

pub struct VideoArea {
    /// Root widget returned to the window layout.
    stack: Stack,
    // ── audio page ────────────────────────────────────────────────────────
    audio_cover: Image,
    audio_title: Label,
    audio_artist: Label,
    audio_album: Label,
    // ── video page ────────────────────────────────────────────────────────
    idle_label: Label,
}

impl VideoArea {
    pub fn new(state: SharedState) -> Self {
        // ── Video page ────────────────────────────────────────────────────
        let gl_area = GLArea::builder()
            .hexpand(true)
            .vexpand(true)
            .build();

        let idle_label = Label::builder()
            .label("Open a file to start playing")
            .css_classes(vec!["dim-label"])
            .build();

        let video_overlay = Overlay::builder()
            .child(&gl_area)
            .hexpand(true)
            .vexpand(true)
            .build();
        video_overlay.add_overlay(&idle_label);

        // ── Audio page ────────────────────────────────────────────────────
        let audio_cover = Image::builder()
            .icon_name("audio-x-generic-symbolic")
            .pixel_size(128)
            .css_classes(vec!["audio-cover-icon"])
            .build();

        let audio_title = Label::builder()
            .label("Unknown Track")
            .css_classes(vec!["title-2"])
            .justify(gtk::Justification::Center)
            .wrap(true)
            .max_width_chars(40)
            .build();

        let audio_artist = Label::builder()
            .label("")
            .css_classes(vec!["dim-label"])
            .justify(gtk::Justification::Center)
            .wrap(true)
            .max_width_chars(40)
            .build();

        let audio_album = Label::builder()
            .label("")
            .css_classes(vec!["caption", "dim-label"])
            .justify(gtk::Justification::Center)
            .wrap(true)
            .max_width_chars(40)
            .build();

        let meta_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .build();
        meta_box.append(&audio_cover);
        meta_box.append(&audio_title);
        meta_box.append(&audio_artist);
        meta_box.append(&audio_album);

        // Center the meta box in the available space
        let audio_page = Box::builder()
            .orientation(Orientation::Vertical)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .vexpand(true)
            .build();
        audio_page.append(&meta_box);

        // ── Stack: "video" | "audio" ──────────────────────────────────────
        let stack = Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(&video_overlay, Some("video"));
        stack.add_named(&audio_page, Some("audio"));
        stack.set_visible_child_name("video");

        // ── Wakeup flag ───────────────────────────────────────────────────
        let needs_render = Arc::new(AtomicBool::new(false));

        // ── realize ───────────────────────────────────────────────────────
        {
            let state_c = state.clone();
            let flag_c = needs_render.clone();
            gl_area.connect_realize(move |area| {
                area.make_current();
                if let Some(err) = area.error() {
                    log::error!("GLArea realize error: {err}");
                    return;
                }

                let render_ctx = {
                    let mut s = state_c.borrow_mut();
                    if let Some(ref mut player) = s.player {
                        match player.create_render_context() {
                            Ok(ctx) => {
                                log::info!("render context created successfully");
                                Some(ctx)
                            }
                            Err(e) => {
                                log::error!("create_render_context failed: {e}");
                                None
                            }
                        }
                    } else {
                        log::error!("GLArea realized but player is None");
                        None
                    }
                };

                if let Some(mut ctx) = render_ctx {
                    let flag = flag_c.clone();
                    ctx.set_update_callback(move || {
                        flag.store(true, Ordering::Relaxed);
                    });
                    state_c.borrow_mut().render_ctx = Some(ctx);

                    // Now that the render context exists, open any session-restore file.
                    let pending = state_c.borrow_mut().pending_open.take();
                    if let Some(path) = pending {
                        if let Some(p) = state_c.borrow().player.as_ref() {
                            p.execute(crate::player::PlayerCommand::Open(path)).ok();
                        }
                    }
                }
            });
        }

        // ── Wakeup timer ──────────────────────────────────────────────────
        {
            let flag_c = needs_render.clone();
            let gl_area_weak = gl_area.downgrade();
            glib::timeout_add_local(Duration::from_millis(8), move || {
                if flag_c.swap(false, Ordering::Relaxed) {
                    if let Some(a) = gl_area_weak.upgrade() {
                        a.queue_render();
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        // ── render ────────────────────────────────────────────────────────
        {
            let state_c = state.clone();
            gl_area.connect_render(move |area, _gl_ctx| {
                let s = state_c.borrow();
                if let Some(ctx) = s.render_ctx.as_ref() {
                    let fbo = current_fbo();
                    let w = area.width();
                    let h = area.height();
                    if let Err(e) = ctx.render(fbo, w, h, true) {
                        log::error!("mpv render: {e}");
                    }
                    ctx.report_swap();
                }
                glib::Propagation::Stop
            });
        }

        // ── unrealize ─────────────────────────────────────────────────────
        {
            let state_c = state.clone();
            gl_area.connect_unrealize(move |_| {
                state_c.borrow_mut().render_ctx = None;
            });
        }

        Self {
            stack,
            audio_cover,
            audio_title,
            audio_artist,
            audio_album,
            idle_label,
        }
    }

    pub fn widget(&self) -> &Stack {
        &self.stack
    }

    /// Switch to the video/idle page.
    pub fn show_video(&self) {
        self.stack.set_visible_child_name("video");
    }

    /// Switch to the audio-info page and populate the metadata labels.
    pub fn show_audio(&self, title: &str, artist: &str, album: &str) {
        self.stack.set_visible_child_name("audio");
        self.audio_title.set_label(if title.is_empty() { "Unknown Track" } else { title });
        self.audio_artist.set_label(artist);
        self.audio_artist.set_visible(!artist.is_empty());
        self.audio_album.set_label(album);
        self.audio_album.set_visible(!album.is_empty());
    }

    /// Show or hide the "open a file" placeholder on the video page.
    pub fn set_idle(&self, idle: bool) {
        self.idle_label.set_visible(idle);
    }
}
