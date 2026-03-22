use std::rc::Rc;
use std::cell::RefCell;
use std::path::PathBuf;

use crate::player::{MpvPlayer, RenderContext};
use crate::player::RepeatMode;

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
    pub repeat_mode: RepeatMode,
    pub shuffle: bool,
    pub shuffle_order: Vec<usize>,
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
            repeat_mode: RepeatMode::default(),
            shuffle: false,
            shuffle_order: Vec::new(),
        }))
    }

    /// Fisher-Yates shuffle using a simple LCG (no external rand dependency).
    pub fn rebuild_shuffle_order(&mut self) {
        let n = self.playlist.len();
        if n == 0 { self.shuffle_order.clear(); return; }
        let mut order: Vec<usize> = (0..n).collect();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(12345);
        let mut rng = seed.wrapping_add(1);
        for i in (1..n).rev() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = ((rng >> 33) as usize) % (i + 1);
            order.swap(i, j);
        }
        // Keep current track at the front of the shuffle
        if let Some(cur) = self.current_idx {
            if let Some(pos) = order.iter().position(|&x| x == cur) {
                order.remove(pos);
                order.insert(0, cur);
            }
        }
        self.shuffle_order = order;
    }

    /// Returns the effective next playlist index, respecting shuffle order.
    pub fn effective_next_idx(&self, current: usize) -> Option<usize> {
        if !self.shuffle {
            let next = current + 1;
            if next < self.playlist.len() { Some(next) } else { None }
        } else {
            let pos = self.shuffle_order.iter().position(|&x| x == current)?;
            self.shuffle_order.get(pos + 1).copied()
        }
    }

    /// Path to the session file in the user data directory.
    pub fn session_path() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("aurora-media").join("session.json"))
    }
}
