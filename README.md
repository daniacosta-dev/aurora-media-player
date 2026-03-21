# Aurora Media Player

Modern video and audio player for GNOME — part of the Aurora suite.

Built with **Rust + GTK4 + libadwaita + libmpv**.

## Dependencies

```bash
# Arch
sudo pacman -S mpv gtk4 libadwaita

# Ubuntu/Debian
sudo apt install libmpv-dev libgtk-4-dev libadwaita-1-dev
```

## Build

```bash
cargo build
cargo run
```

## Architecture

```
app/src/
├── main.rs          # Entry point
├── app.rs           # GtkApplication setup
├── player/
│   ├── mpv.rs       # libmpv wrapper
│   └── pipeline.rs  # PlaybackState / PlayerCommand enums
├── ui/
│   ├── window.rs    # AdwApplicationWindow root
│   ├── headerbar.rs # AdwHeaderBar with actions
│   ├── video_area.rs# Drawing surface for mpv rendering
│   ├── controls.rs  # Seek bar + playback buttons
│   └── playlist.rs  # Sidebar playlist panel
└── library/
    └── scanner.rs   # Recursive media file scanner
```
