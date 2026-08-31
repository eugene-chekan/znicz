use std::io::Read;
use std::path::{Path, PathBuf};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecParameters, CodecType, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::error::{Result, ZniczError};
use crate::metadata::{read_metadata, title_from_path};
use crate::player::state::TrackInfo;

/// Build track info from the decoder's view of the file plus its tags.
fn track_info(path: &Path, codec_params: &CodecParameters) -> TrackInfo {
    let meta = read_metadata(path);
    let title = meta
        .tags
        .title
        .clone()
        .unwrap_or_else(|| title_from_path(path));

    let duration = codec_params
        .n_frames
        .and_then(|frames| {
            codec_params
                .sample_rate
                .map(|rate| std::time::Duration::from_secs_f64(frames as f64 / rate as f64))
        })
        .or(meta.properties.duration);

    let sample_rate = codec_params.sample_rate.unwrap_or(0);
    let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(0);
    let bits_per_sample = codec_params
        .bits_per_sample
        .or(meta.properties.bits_per_sample);
    let pcm = is_pcm(codec_params.codec);

    let bitrate_kbps = meta.properties.audio_bitrate.or_else(|| {
        // Uncompressed PCM has a known rate: samples × channels × bits.
        if pcm {
            uncompressed_bitrate_kbps(sample_rate, channels, bits_per_sample)
        } else {
            None
        }
    });

    TrackInfo {
        path: Some(path.to_path_buf()),
        url: None,
        title,
        codec: codec_label(codec_params.codec, path),
        sample_rate,
        channels,
        bits_per_sample,
        bitrate_kbps,
        duration,
        tags: meta.tags,
    }
}

/// A name a person would recognise, never a hex codec id such as `0x1003`.
fn codec_label(codec: CodecType, path: &Path) -> String {
    use symphonia::core::codecs::*;

    if is_pcm(codec) {
        return match extension_lower(path).as_deref() {
            Some("wav") => "WAV".into(),
            Some("aiff" | "aif" | "aifc" | "afc") => "AIFF".into(),
            Some("caf") => "CAF".into(),
            _ => "PCM".into(),
        };
    }

    let name = match codec {
        CODEC_TYPE_MP3 => "MP3",
        CODEC_TYPE_MP2 => "MP2",
        CODEC_TYPE_MP1 => "MP1",
        CODEC_TYPE_AAC => "AAC",
        CODEC_TYPE_OPUS => "Opus",
        CODEC_TYPE_VORBIS => "Vorbis",
        CODEC_TYPE_SPEEX => "Speex",
        CODEC_TYPE_FLAC => "FLAC",
        CODEC_TYPE_ALAC => "ALAC",
        CODEC_TYPE_WAVPACK => "WavPack",
        CODEC_TYPE_MONKEYS_AUDIO => "APE",
        CODEC_TYPE_TTA => "TTA",
        CODEC_TYPE_MUSEPACK => "Musepack",
        CODEC_TYPE_WMA => "WMA",
        CODEC_TYPE_EAC3 => "E-AC-3",
        CODEC_TYPE_AC4 => "AC-4",
        CODEC_TYPE_DCA => "DTS",
        CODEC_TYPE_ATRAC1 => "ATRAC1",
        CODEC_TYPE_ATRAC3 => "ATRAC3",
        CODEC_TYPE_ATRAC3PLUS => "ATRAC3+",
        CODEC_TYPE_ATRAC9 => "ATRAC9",
        CODEC_TYPE_ADPCM_G722
        | CODEC_TYPE_ADPCM_G726
        | CODEC_TYPE_ADPCM_G726LE
        | CODEC_TYPE_ADPCM_MS
        | CODEC_TYPE_ADPCM_IMA_WAV
        | CODEC_TYPE_ADPCM_IMA_QT => "ADPCM",
        _ => {
            return extension_lower(path)
                .map(|ext| ext.to_ascii_uppercase())
                .unwrap_or_else(|| "Audio".into());
        }
    };
    name.to_string()
}

fn is_pcm(codec: CodecType) -> bool {
    use symphonia::core::codecs::*;
    matches!(
        codec,
        CODEC_TYPE_PCM_S32LE
            | CODEC_TYPE_PCM_S32LE_PLANAR
            | CODEC_TYPE_PCM_S32BE
            | CODEC_TYPE_PCM_S32BE_PLANAR
            | CODEC_TYPE_PCM_S24LE
            | CODEC_TYPE_PCM_S24LE_PLANAR
            | CODEC_TYPE_PCM_S24BE
            | CODEC_TYPE_PCM_S24BE_PLANAR
            | CODEC_TYPE_PCM_S16LE
            | CODEC_TYPE_PCM_S16LE_PLANAR
            | CODEC_TYPE_PCM_S16BE
            | CODEC_TYPE_PCM_S16BE_PLANAR
            | CODEC_TYPE_PCM_S8
            | CODEC_TYPE_PCM_S8_PLANAR
            | CODEC_TYPE_PCM_U32LE
            | CODEC_TYPE_PCM_U32LE_PLANAR
            | CODEC_TYPE_PCM_U32BE
            | CODEC_TYPE_PCM_U32BE_PLANAR
            | CODEC_TYPE_PCM_U24LE
            | CODEC_TYPE_PCM_U24LE_PLANAR
            | CODEC_TYPE_PCM_U24BE
            | CODEC_TYPE_PCM_U24BE_PLANAR
            | CODEC_TYPE_PCM_U16LE
            | CODEC_TYPE_PCM_U16LE_PLANAR
            | CODEC_TYPE_PCM_U16BE
            | CODEC_TYPE_PCM_U16BE_PLANAR
            | CODEC_TYPE_PCM_U8
            | CODEC_TYPE_PCM_U8_PLANAR
            | CODEC_TYPE_PCM_F32LE
            | CODEC_TYPE_PCM_F32LE_PLANAR
            | CODEC_TYPE_PCM_F32BE
            | CODEC_TYPE_PCM_F32BE_PLANAR
            | CODEC_TYPE_PCM_F64LE
            | CODEC_TYPE_PCM_F64LE_PLANAR
            | CODEC_TYPE_PCM_F64BE
            | CODEC_TYPE_PCM_F64BE_PLANAR
            | CODEC_TYPE_PCM_ALAW
            | CODEC_TYPE_PCM_MULAW
    )
}

fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

fn uncompressed_bitrate_kbps(sample_rate: u32, channels: u16, bits: Option<u32>) -> Option<u32> {
    let bits = bits?;
    if sample_rate == 0 || channels == 0 || bits == 0 {
        return None;
    }
    Some((u64::from(sample_rate) * u64::from(channels) * u64::from(bits) / 1000) as u32)
}

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

    Ok(track_info(path, &track.codec_params))
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

        let track_info = track_info(path, &codec_params);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use symphonia::core::codecs::{CODEC_TYPE_FLAC, CODEC_TYPE_MP3, CODEC_TYPE_PCM_S16LE};

    #[test]
    fn codec_ids_become_format_names() {
        assert_eq!(
            codec_label(CODEC_TYPE_MP3, Path::new("song.mp3")),
            "MP3",
            "0x1003 is MPEG layer 3"
        );
        assert_eq!(
            codec_label(CODEC_TYPE_FLAC, Path::new("song.flac")),
            "FLAC",
            "0x2000 is FLAC"
        );
        assert_eq!(
            codec_label(CODEC_TYPE_PCM_S16LE, Path::new("song.wav")),
            "WAV",
            "PCM inside a WAVE file is shown as WAV"
        );
        assert_eq!(
            codec_label(CODEC_TYPE_PCM_S16LE, Path::new("song.aiff")),
            "AIFF"
        );
    }

    #[test]
    fn uncompressed_cd_audio_is_1411_kbps() {
        assert_eq!(uncompressed_bitrate_kbps(44_100, 2, Some(16)), Some(1411));
    }
}
