mod app;
mod player;
mod state;
mod ui;
mod library;

use app::AuroraMediaApp;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    AuroraMediaApp::run()
}
