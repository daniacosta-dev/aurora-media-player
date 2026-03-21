use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MediaKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    pub path: PathBuf,
    pub title: String,
    pub kind: MediaKind,
    pub duration_secs: Option<f64>,
}

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "ts",
];

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "aac", "m4a", "wav", "wma",
];

/// Scans a directory recursively for media files.
pub fn scan_directory(dir: &Path) -> Vec<MediaItem> {
    let mut items = Vec::new();
    scan_recursive(dir, &mut items);
    items
}

fn scan_recursive(dir: &Path, out: &mut Vec<MediaItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_recursive(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext_lower = ext.to_lowercase();
            let kind = if VIDEO_EXTENSIONS.contains(&ext_lower.as_str()) {
                MediaKind::Video
            } else if AUDIO_EXTENSIONS.contains(&ext_lower.as_str()) {
                MediaKind::Audio
            } else {
                continue;
            };
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();
            out.push(MediaItem {
                path,
                title,
                kind,
                duration_secs: None,
            });
        }
    }
}
