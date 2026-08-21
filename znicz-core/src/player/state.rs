use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub path: PathBuf,
    pub title: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: Option<u32>,
    pub duration: Option<Duration>,
}

impl TrackInfo {
    pub fn format_description(&self) -> String {
        let bits = self
            .bits_per_sample
            .map(|b| format!("{}-bit", b))
            .unwrap_or_else(|| "unknown depth".to_string());
        format!(
            "{} {}kHz/{} ch",
            self.codec,
            self.sample_rate / 1000,
            bits
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub status: PlaybackStatus,
    pub current_track: Option<TrackInfo>,
    pub position: Duration,
    pub volume: f32,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub queue: Vec<PathBuf>,
    pub queue_position: usize,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            current_track: None,
            position: Duration::ZERO,
            volume: 1.0,
            device_id: None,
            device_name: None,
            queue: Vec::new(),
            queue_position: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
