use adw::prelude::*;
use adw::Application;
use gtk4 as gtk;
use gtk4::prelude::StyleContextExt;
use gio::ApplicationFlags;
use gdk4::Display;

use crate::ui::window::MediaWindow;

const APP_ID: &str = "io.github.aurora.MediaPlayer";

// Embed and register the compiled gresource bundle at startup.
fn init_resources() {
    gio::resources_register_include!("aurora-media.gresource")
        .expect("Failed to register gresources");
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_resource("/io/github/aurora/MediaPlayer/style.css");
    if let Some(display) = Display::default() {
        gtk::StyleContext::add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

pub struct AuroraMediaApp;

impl AuroraMediaApp {
    pub fn run() -> anyhow::Result<()> {
        init_resources();

        let app = Application::builder()
            .application_id(APP_ID)
            .flags(ApplicationFlags::HANDLES_OPEN)
            .build();

        app.connect_startup(|_| {
            adw::init().expect("Failed to initialize libadwaita");
            gtk::Window::set_default_icon_name(APP_ID);
            load_css();
        });

        app.connect_activate(|app| {
            let window = MediaWindow::new(app);
            window.present();
        });

        // Handle file opens from CLI or file manager
        app.connect_open(|app, files, _hint| {
            let window = MediaWindow::new(app);
            window.present();
            if let Some(file) = files.first() {
                if let Some(path) = file.path() {
                    window.open_file(&path);
                }
            }
        });

        let exit_code = app.run();
        if exit_code != glib::ExitCode::SUCCESS {
            anyhow::bail!("Application exited with error");
        }
        Ok(())
    }
}
