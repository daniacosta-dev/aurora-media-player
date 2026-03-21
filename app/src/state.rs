use std::rc::Rc;
use std::cell::RefCell;
use std::path::PathBuf;

use crate::player::{MpvPlayer, RenderContext};

pub struct PlayerState {
    pub player: Option<MpvPlayer>,
    /// OpenGL render context — lives alongside the player.
    pub render_ctx: Option<RenderContext>,
    pub playlist: Vec<PathBuf>,
    pub current_idx: Option<usize>,
    pub muted: bool,
    /// When set, the polling loop will seek to this position once the file loads.
    pub pending_seek: Option<f64>,
    /// File to open once the GL render context is ready (session restore).
    pub pending_open: Option<PathBuf>,
}

/// Single-threaded shared handle used throughout the UI tree.
pub type SharedState = Rc<RefCell<PlayerState>>;

impl PlayerState {
    pub fn create() -> SharedState {
        let player = MpvPlayer::new()
            .map_err(|e| log::error!("mpv init failed: {e}"))
            .ok();
        Rc::new(RefCell::new(Self {
            player,
            render_ctx: None,
            playlist: Vec::new(),
            current_idx: None,
            muted: false,
            pending_seek: None,
            pending_open: None,
        }))
    }

    /// Path to the session file in the user data directory.
    pub fn session_path() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("aurora-media").join("session.json"))
    }
}
