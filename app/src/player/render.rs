/// Safe wrapper around the libmpv OpenGL render API.
///
/// The render context must be created and used exclusively from the thread
/// that owns the OpenGL context (GTK main thread in our case).
use anyhow::{bail, Result};
use std::os::raw::{c_char, c_int, c_void};

use libmpv_sys::{
    mpv_handle,
    mpv_opengl_fbo,
    mpv_opengl_init_params,
    mpv_render_context,
    mpv_render_context_create,
    mpv_render_context_free,
    mpv_render_context_render,
    mpv_render_context_report_swap,
    mpv_render_context_set_update_callback,
    mpv_render_param,
    mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
    mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
    mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
    mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
    MPV_RENDER_API_TYPE_OPENGL,
};

/// Resolve a GL function pointer.
///
/// GTK4 initialises epoxy / the GL driver before our code runs, so all
/// standard GL symbols are already loaded into the process.
/// `dlsym(RTLD_DEFAULT, name)` finds them without linking against a specific
/// GL library — the right choice for both GLX (X11) and EGL (Wayland).
unsafe extern "C" fn get_proc_addr(
    _ctx: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    libc::dlsym(libc::RTLD_DEFAULT, name)
}

/// Boxed callback stored alongside the context so it is freed on drop.
type Callback = Box<dyn Fn() + Send + 'static>;

/// Safe wrapper around `mpv_render_context`.
/// NOT `Send` — must stay on the GL thread.
pub struct RenderContext {
    ctx: *mut mpv_render_context,
    /// Keeps the wakeup callback alive for the lifetime of this struct.
    _cb: Option<Box<Callback>>,
}

// SAFETY: We ensure all calls happen on the GTK main thread.
// The Send impl is required so we can store it in RefCell<PlayerState>
// which is accessed from async GLib closures. All actual use is single-threaded.
unsafe impl Send for RenderContext {}

impl RenderContext {
    /// Create an OpenGL render context.
    /// **Must be called while the target GL context is current.**
    pub fn new(mpv: *mut mpv_handle) -> Result<Self> {
        let mut init_params = mpv_opengl_init_params {
            get_proc_address: Some(get_proc_addr),
            get_proc_address_ctx: std::ptr::null_mut(),
        };

        let api_type_ptr = MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut c_void;

        let mut params = [
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: api_type_ptr,
            },
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init_params as *mut _ as *mut c_void,
            },
            mpv_render_param {
                type_: 0,
                data: std::ptr::null_mut(),
            },
        ];

        let mut ctx: *mut mpv_render_context = std::ptr::null_mut();
        let ret = unsafe {
            mpv_render_context_create(&mut ctx, mpv, params.as_mut_ptr())
        };

        if ret != 0 {
            bail!("mpv_render_context_create failed: code {ret}");
        }
        Ok(Self { ctx, _cb: None })
    }

    /// Register the wakeup callback.  Called by mpv from an internal thread
    /// when a new frame is ready; do **not** call any mpv API inside it.
    pub fn set_update_callback<F: Fn() + Send + 'static>(&mut self, cb: F) {
        // Heap-allocate the closure so we can pass a raw pointer to C.
        let boxed: Box<Callback> = Box::new(Box::new(cb));
        let ptr = Box::into_raw(boxed);

        unsafe extern "C" fn trampoline(ctx: *mut c_void) {
            // SAFETY: We created this pointer in set_update_callback.
            let cb = &*(ctx as *const Callback);
            cb();
        }

        unsafe {
            mpv_render_context_set_update_callback(
                self.ctx,
                Some(trampoline),
                ptr as *mut c_void,
            );
        }

        // Reclaim ownership so it is freed when `self` is dropped.
        self._cb = Some(unsafe { Box::from_raw(ptr) });
    }

    /// Render the current frame into the given OpenGL framebuffer.
    /// **Must be called with the GL context current.**
    pub fn render(&self, fbo: c_int, w: c_int, h: c_int, flip_y: bool) -> Result<()> {
        let mut fbo_params = mpv_opengl_fbo {
            fbo,
            w,
            h,
            internal_format: 0,
        };
        let mut flip: c_int = flip_y as c_int;

        let mut params = [
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                data: &mut fbo_params as *mut _ as *mut c_void,
            },
            mpv_render_param {
                type_: mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
                data: &mut flip as *mut _ as *mut c_void,
            },
            mpv_render_param {
                type_: 0,
                data: std::ptr::null_mut(),
            },
        ];

        let ret = unsafe { mpv_render_context_render(self.ctx, params.as_mut_ptr()) };
        if ret != 0 {
            bail!("mpv_render_context_render failed: code {ret}");
        }
        Ok(())
    }

    /// Notify mpv that the rendered frame has been presented.
    pub fn report_swap(&self) {
        unsafe { mpv_render_context_report_swap(self.ctx) };
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // Clear the callback BEFORE freeing the context so mpv stops calling it.
        unsafe {
            mpv_render_context_set_update_callback(self.ctx, None, std::ptr::null_mut());
        }
        unsafe { mpv_render_context_free(self.ctx) };
        // _cb is dropped here, freeing the boxed closure.
    }
}
