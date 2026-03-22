mod app;
mod mpris;
mod player;
mod state;
mod ui;
mod library;

use app::AuroraMediaApp;

fn main() -> anyhow::Result<()> {
    env_logger::init();

    // Install dev-time icon + .desktop so the compositor can find them
    // before the Wayland surface is created.  In a snap/flatpak the source
    // asset won't exist so this block is a no-op.
    install_dev_assets();

    AuroraMediaApp::run()
}

fn install_dev_assets() {
    let icon_src = std::env::current_dir()
        .unwrap_or_default()
        .join("app/src/assets/aurora_media_player_icon.svg");

    if !icon_src.exists() {
        return;
    }

    let Some(data_dir) = dirs::data_dir() else { return };

    // Icon → ~/.local/share/icons/hicolor/scalable/apps/
    let icon_dir = data_dir.join("icons/hicolor/scalable/apps");
    std::fs::create_dir_all(&icon_dir).ok();
    std::fs::copy(
        &icon_src,
        icon_dir.join("io.github.daniacosta_dev.AuroraMediaPlayer.svg"),
    )
    .ok();

    // .desktop → ~/.local/share/applications/
    // GNOME Shell uses this to resolve the window app-id → icon name.
    let apps_dir = data_dir.join("applications");
    std::fs::create_dir_all(&apps_dir).ok();
    std::fs::write(
        apps_dir.join("io.github.daniacosta_dev.AuroraMediaPlayer.desktop"),
        "[Desktop Entry]\n\
         Name=Aurora Media Player\n\
         Exec=aurora-media %U\n\
         Icon=io.github.daniacosta_dev.AuroraMediaPlayer\n\
         Type=Application\n\
         MimeType=video/mp4;video/x-matroska;audio/mpeg;audio/flac;\n",
    )
    .ok();
}
