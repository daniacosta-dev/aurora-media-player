#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackState {
    Idle,
    Loading,
    Playing,
    Paused,
    Stopped,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum PlayerCommand {
    Open(std::path::PathBuf),
    Play,
    Pause,
    TogglePause,
    Stop,
    Seek(f64),           // seconds
    SetVolume(f64),      // 0.0 - 100.0
    Mute(bool),
    SetSpeed(f64),       // 0.25 - 4.0
    NextFrame,
    PrevFrame,
    Screenshot,
}
