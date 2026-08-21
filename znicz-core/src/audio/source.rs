use std::io::Read;
use std::path::{Path, PathBuf};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::error::{Result, ZniczError};
use crate::player::state::TrackInfo;

fn format_options() -> FormatOptions {
    FormatOptions {
        enable_gapless: true,
        ..Default::default()
    }
}

pub trait AudioSource: Send {
    fn path(&self) -> &Path;
    fn read_info(&self) -> Result<TrackInfo>;
    fn open_reader(&self) -> Result<Box<dyn Read + Send>>;
}

#[derive(Debug, Clone)]
pub struct LocalFileSource {
    path: PathBuf,
}

impl LocalFileSource {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl AudioSource for LocalFileSource {
    fn path(&self) -> &Path {
        &self.path
    }

    fn read_info(&self) -> Result<TrackInfo> {
        probe_track(&self.path)
    }

    fn open_reader(&self) -> Result<Box<dyn Read + Send>> {
        let file = std::fs::File::open(&self.path)?;
        Ok(Box::new(file))
    }
}

pub fn probe_track(path: &Path) -> Result<TrackInfo> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_options(), &MetadataOptions::default())
        .map_err(|e| ZniczError::Decode(e.to_string()))?;

    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .or_else(|| probed.format.tracks().first())
        .ok_or_else(|| ZniczError::Decode("no audio track".into()))?;

    let codec_params = track.codec_params.clone();
    let codec = codec_params.codec.to_string();
    let sample_rate = codec_params.sample_rate.unwrap_or(0);
    let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(0);
    let bits_per_sample = codec_params.bits_per_sample;
    let duration = codec_params
        .n_frames
        .and_then(|frames| codec_params.sample_rate.map(|rate| {
            std::time::Duration::from_secs_f64(frames as f64 / rate as f64)
        }));

    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    Ok(TrackInfo {
        path: path.to_path_buf(),
        title,
        codec,
        sample_rate,
        channels,
        bits_per_sample,
        duration,
    })
}

pub struct AudioDecoder {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
}

impl AudioDecoder {
    pub fn open(path: &Path) -> Result<(Self, TrackInfo)> {
        let file = std::fs::File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_options(), &MetadataOptions::default())
            .map_err(|e| ZniczError::Decode(e.to_string()))?;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .or_else(|| probed.format.tracks().first())
            .ok_or_else(|| ZniczError::Decode("no audio track".into()))?;

        let track_id = track.id;
        let codec_params = track.codec_params.clone();
        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);

        let decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .map_err(|e| ZniczError::Decode(e.to_string()))?;

        let track_info = TrackInfo {
            path: path.to_path_buf(),
            title: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string(),
            codec: codec_params.codec.to_string(),
            sample_rate,
            channels,
            bits_per_sample: codec_params.bits_per_sample,
            duration: codec_params
                .n_frames
                .and_then(|frames| codec_params.sample_rate.map(|rate| {
                    std::time::Duration::from_secs_f64(frames as f64 / rate as f64)
                })),
        };

        Ok((
            Self {
                format: probed.format,
                decoder,
                track_id,
                sample_rate,
                channels,
            },
            track_info,
        ))
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn seek(&mut self, position: std::time::Duration) -> Result<()> {
        let time = Time::from(position);
        self.format
            .seek(
                symphonia::core::formats::SeekMode::Accurate,
                symphonia::core::formats::SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| ZniczError::Decode(e.to_string()))?;
        Ok(())
    }

    /// Decode the next packet into interleaved f32 samples.
    pub fn decode_next(&mut self) -> Result<Option<Vec<f32>>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::ResetRequired) => {
                    return Err(ZniczError::Decode("stream reset required".into()));
                }
                Err(SymphoniaError::IoError(_)) => {
                    return Ok(None);
                }
                Err(e) => {
                    if e.to_string().contains("end of stream") {
                        return Ok(None);
                    }
                    return Err(ZniczError::Decode(e.to_string()));
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let duration = decoded.capacity() as u64;
                    let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);
                    sample_buf.copy_interleaved_ref(decoded);
                    return Ok(Some(sample_buf.samples().to_vec()));
                }
                Err(SymphoniaError::IoError(_)) | Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(ZniczError::Decode(e.to_string())),
            }
        }
    }
}
