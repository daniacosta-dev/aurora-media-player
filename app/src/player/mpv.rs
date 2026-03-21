use std::ffi::CString;
use anyhow::Result;
use libmpv::Mpv;
use libmpv_sys;
use libc;

use super::pipeline::PlayerCommand;
use super::render::RenderContext;

/// Call mpv_command using the safe null-terminated pointer array API.
/// This handles paths with spaces and special characters, unlike mpv_command_string.
fn mpv_command_array(ctx: *mut libmpv_sys::mpv_handle, args: &[&str]) -> Result<()> {
    let cstrings: Vec<CString> = args
        .iter()
        .map(|s| CString::new(*s).map_err(|e| anyhow::anyhow!("{e}")))
        .collect::<Result<_>>()?;
    let mut ptrs: Vec<*const libc::c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    let ret = unsafe { libmpv_sys::mpv_command(ctx, ptrs.as_mut_ptr()) };
    if ret != 0 {
        anyhow::bail!("mpv_command failed: code {ret}");
    }
    Ok(())
}

pub struct MpvPlayer {
    pub(crate) mpv: Mpv,
}

impl MpvPlayer {
    /// Create a new mpv instance.  Video is rendered via the OpenGL render
    /// API — no window ID required.
    pub fn new() -> Result<Self> {
        // GTK resets LC_ALL; restore LC_NUMERIC=C before mpv_create().
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, b"C\0".as_ptr() as *const libc::c_char);
        }

        let mpv = Mpv::with_initializer(|init| {
            // Must be set before mpv_initialize() so the render API works.
            init.set_property("vo", "libmpv")?;
            Ok(())
        })
        .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        mpv.set_property("hwdec", "auto-safe").ok();
        mpv.set_property("volume", 100.0_f64).ok();
        mpv.set_property("keep-open", "yes").ok();

        // Disable mpv's built-in OSD/input — we build our own.
        mpv.set_property("osc", false).ok();
        mpv.set_property("input-default-bindings", false).ok();
        mpv.set_property("input-vo-keyboard", false).ok();

        Ok(Self { mpv })
    }

    /// Create an OpenGL render context for this player.
    /// **Must be called while a GL context is current.**
    pub fn create_render_context(&mut self) -> Result<RenderContext> {
        // SAFETY: ctx is valid as long as self.mpv is alive.
        // We keep both together in PlayerState.
        RenderContext::new(unsafe { self.mpv.ctx.as_ptr() })
    }

    pub fn execute(&self, cmd: PlayerCommand) -> Result<()> {
        match cmd {
            PlayerCommand::Open(path) => {
                let path_str = path.to_str().ok_or_else(|| anyhow::anyhow!("non-UTF8 path"))?;
                mpv_command_array(self.mpv.ctx.as_ptr(), &["loadfile", path_str, "replace"])?;
                self.mpv.set_property("pause", false).ok();
            }
            PlayerCommand::Play => {
                self.mpv.set_property("pause", false).ok();
            }
            PlayerCommand::Pause => {
                self.mpv.set_property("pause", true).ok();
            }
            PlayerCommand::TogglePause => {
                self.mpv.command("cycle", &["pause"]).ok();
            }
            PlayerCommand::Stop => {
                self.mpv.command("stop", &[]).ok();
            }
            PlayerCommand::Seek(secs) => {
                self.mpv
                    .command("seek", &[&secs.to_string(), "absolute"])
                    .ok();
            }
            PlayerCommand::SetVolume(vol) => {
                self.mpv.set_property("volume", vol).ok();
            }
            PlayerCommand::Mute(muted) => {
                self.mpv.set_property("mute", muted).ok();
            }
            PlayerCommand::SetSpeed(speed) => {
                self.mpv.set_property("speed", speed).ok();
            }
            PlayerCommand::NextFrame => {
                self.mpv.command("frame-step", &[]).ok();
            }
            PlayerCommand::PrevFrame => {
                self.mpv.command("frame-back-step", &[]).ok();
            }
            PlayerCommand::Screenshot => {
                self.mpv.command("screenshot", &[]).ok();
            }
        }
        Ok(())
    }

    pub fn duration(&self) -> Option<f64> {
        self.mpv.get_property("duration").ok()
    }

    pub fn position(&self) -> Option<f64> {
        self.mpv.get_property("time-pos").ok()
    }

    pub fn volume(&self) -> f64 {
        self.mpv.get_property("volume").unwrap_or(100.0)
    }

    pub fn is_paused(&self) -> bool {
        self.mpv.get_property("pause").unwrap_or(true)
    }

    pub fn is_muted(&self) -> bool {
        self.mpv.get_property("mute").unwrap_or(false)
    }

    pub fn media_title(&self) -> Option<String> {
        self.mpv.get_property("media-title").ok()
    }

    pub fn is_idle(&self) -> bool {
        self.mpv
            .get_property::<bool>("idle-active")
            .unwrap_or(true)
    }

    /// Returns true when the current file has a real video track.
    /// Audio-only files (mp3, flac, ogg…) return false.
    /// Note: mpv also exposes embedded album art as a video track, so files
    /// with cover art embedded will return true and render through the GLArea,
    /// which is exactly what we want.
    pub fn has_video(&self) -> bool {
        self.mpv
            .get_property::<i64>("width")
            .map(|w| w > 0)
            .unwrap_or(false)
    }

    pub fn metadata_artist(&self) -> Option<String> {
        self.mpv
            .get_property("metadata/by-key/Artist")
            .ok()
            .or_else(|| self.mpv.get_property("metadata/by-key/artist").ok())
    }

    pub fn metadata_album(&self) -> Option<String> {
        self.mpv
            .get_property("metadata/by-key/Album")
            .ok()
            .or_else(|| self.mpv.get_property("metadata/by-key/album").ok())
    }
}
