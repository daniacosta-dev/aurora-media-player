use std::rc::Rc;
use std::cell::RefCell;
use std::path::PathBuf;

use crate::player::{MpvPlayer, RenderContext};

pub struct PlayerState {
    pub player: Option<MpvPlayer>,
    /// OpenGL render context — lives alongside the player.
    /// Created in GLArea::realize and dropped in GLArea::unrealize.
    pub render_ctx: Option<RenderContext>,
    pub playlist: Vec<PathBuf>,
    pub current_idx: Option<usize>,
    pub muted: bool,
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
        }))
    }
}
