use adw::prelude::*;
use adw::{Application, ApplicationWindow};
use gtk4 as gtk;
use gio::ApplicationFlags;

use crate::ui::window::MediaWindow;

const APP_ID: &str = "io.github.aurora.MediaPlayer";

pub struct AuroraMediaApp;

impl AuroraMediaApp {
    pub fn run() -> anyhow::Result<()> {
        let app = Application::builder()
            .application_id(APP_ID)
            .flags(ApplicationFlags::HANDLES_OPEN)
            .build();

        app.connect_startup(|_| {
            adw::init().expect("Failed to initialize libadwaita");
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
