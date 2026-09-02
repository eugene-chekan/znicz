use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecParameters, CodecType, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::audio::icy::{IcyTitle, IcyUrl};
use crate::error::{Result, ZniczError};
use crate::metadata::{read_metadata, title_from_path};
use crate::player::state::TrackInfo;

fn track_info_from_params(codec_params: &CodecParameters, source: &dyn AudioSource) -> TrackInfo {
    if let Some(path) = source.path() {
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
            if pcm {
                uncompressed_bitrate_kbps(sample_rate, channels, bits_per_sample)
            } else {
                None
            }
        });

        TrackInfo {
            path: Some(path.to_path_buf()),
            url: None,
            icy_stream_url: None,
            title,
            codec: codec_label(codec_params.codec, path),
            sample_rate,
            channels,
            bits_per_sample,
            bitrate_kbps,
            duration,
            tags: meta.tags,
        }
    } else {
        let sample_rate = codec_params.sample_rate.unwrap_or(0);
        let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(0);
        let bits_per_sample = codec_params.bits_per_sample;
        let pcm = is_pcm(codec_params.codec);

        let bitrate_kbps = if pcm {
            uncompressed_bitrate_kbps(sample_rate, channels, bits_per_sample)
        } else {
            None
        };

        TrackInfo {
            path: None,
            url: source.url().map(str::to_string),
            icy_stream_url: None,
            title: source.title_hint().to_string(),
            codec: codec_label(codec_params.codec, Path::new("")),
            sample_rate,
            channels,
            bits_per_sample,
            bitrate_kbps,
            duration: None,
            tags: Default::default(),
        }
    }
}

/// Build track info from the decoder's view of the file plus its tags.
fn track_info(path: &Path, codec_params: &CodecParameters) -> TrackInfo {
    track_info_from_params(codec_params, &LocalFileSource::new(path.to_path_buf()))
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

/// Coded audio bitrate from bytes fed to the decoder and PCM duration produced.
///
/// Needs at least a quarter second of audio so the first packets do not flash a
/// nonsense number. Used for live radio; files still use tags.
fn coded_bitrate_kbps(coded_bytes: u64, pcm_frames: u64, sample_rate: u32) -> Option<u32> {
    if pcm_frames == 0 || sample_rate == 0 {
        return None;
    }
    let seconds = pcm_frames as f64 / f64::from(sample_rate);
    if seconds < 0.25 {
        return None;
    }
    Some(((coded_bytes as f64 * 8.0) / seconds / 1000.0).round() as u32)
}

fn format_options() -> FormatOptions {
    FormatOptions {
        enable_gapless: true,
        ..Default::default()
    }
}

pub trait AudioSource: Send {
    fn path(&self) -> Option<&Path>;
    fn url(&self) -> Option<&str>;
    fn title_hint(&self) -> &str;
    fn read_info(&self) -> Result<TrackInfo>;
    fn open_reader(&self) -> Result<Box<dyn symphonia::core::io::MediaSource>>;
    fn icy_title_slot(&self) -> Option<Arc<Mutex<IcyTitle>>> {
        None
    }
    fn icy_url_slot(&self) -> Option<Arc<Mutex<IcyUrl>>> {
        None
    }
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
    fn path(&self) -> Option<&Path> {
        Some(&self.path)
    }

    fn url(&self) -> Option<&str> {
        None
    }

    fn title_hint(&self) -> &str {
        self.path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
    }

    fn read_info(&self) -> Result<TrackInfo> {
        probe_track(&self.path)
    }

    fn open_reader(&self) -> Result<Box<dyn symphonia::core::io::MediaSource>> {
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
    /// True when opened from a URL source. Stream body I/O is not EOF.
    is_stream: bool,
    coded_bytes: u64,
    pcm_frames: u64,
    icy_title: Option<Arc<Mutex<IcyTitle>>>,
    icy_url: Option<Arc<Mutex<IcyUrl>>>,
}

impl AudioDecoder {
    pub fn open(source: &dyn AudioSource) -> Result<(Self, TrackInfo)> {
        let icy_title = source.icy_title_slot();
        let icy_url = source.icy_url_slot();
        let reader = source.open_reader()?;
        let mss = MediaSourceStream::new(reader, Default::default());

        let mut hint = Hint::new();
        if let Some(path) = source.path() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                hint.with_extension(ext);
            }
        } else if let Some(url) = source.url() {
            if let Some(ext) = url.rsplit('.').next().filter(|s| s.len() <= 4) {
                hint.with_extension(ext);
            }
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

        let track_info = track_info_from_params(&codec_params, source);
        let is_stream = source.url().is_some();

        Ok((
            Self {
                format: probed.format,
                decoder,
                track_id,
                sample_rate,
                channels,
                is_stream,
                coded_bytes: 0,
                pcm_frames: 0,
                icy_title,
                icy_url,
            },
            track_info,
        ))
    }

    pub fn open_path(path: &Path) -> Result<(Self, TrackInfo)> {
        Self::open(&LocalFileSource::new(path.to_path_buf()))
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn is_stream(&self) -> bool {
        self.is_stream
    }

    /// Live coded bitrate once enough PCM has been decoded. `None` for files
    /// that have not produced a quarter second yet, or for a stream that just
    /// opened.
    pub fn measured_bitrate_kbps(&self) -> Option<u32> {
        coded_bitrate_kbps(self.coded_bytes, self.pcm_frames, self.sample_rate)
    }

    pub fn icy_title(&self) -> IcyTitle {
        self.icy_title
            .as_ref()
            .map(|slot| slot.lock().unwrap().clone())
            .unwrap_or(IcyTitle::Unset)
    }

    pub fn icy_url(&self) -> IcyUrl {
        self.icy_url
            .as_ref()
            .map(|slot| slot.lock().unwrap().clone())
            .unwrap_or(IcyUrl::Unset)
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
                Err(SymphoniaError::IoError(e)) => {
                    if self.is_stream {
                        return Err(ZniczError::Decode(format!("stream io error: {e}")));
                    }
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

            let coded = packet.buf().len() as u64;
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let duration = decoded.capacity() as u64;
                    let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);
                    sample_buf.copy_interleaved_ref(decoded);
                    let samples = sample_buf.samples().to_vec();
                    let channels = self.channels.max(1) as u64;
                    self.coded_bytes += coded;
                    self.pcm_frames += samples.len() as u64 / channels;
                    return Ok(Some(samples));
                }
                Err(SymphoniaError::IoError(e)) => {
                    if self.is_stream {
                        return Err(ZniczError::Decode(format!("stream decode io error: {e}")));
                    }
                    continue;
                }
                Err(SymphoniaError::DecodeError(_)) => continue,
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

    #[test]
    fn coded_bitrate_needs_a_quarter_second_of_pcm() {
        assert_eq!(coded_bitrate_kbps(16_000, 0, 44_100), None);
        assert_eq!(coded_bitrate_kbps(16_000, 100, 44_100), None);
        // 192 kbps CBR: 24_000 bytes over 1 s of 44.1 kHz audio.
        assert_eq!(coded_bitrate_kbps(24_000, 44_100, 44_100), Some(192));
    }

    fn silent_wav_bytes(frames: u32) -> Vec<u8> {
        let channels = 1u16;
        let sample_rate = 44_100u32;
        let bytes_per_frame = 2u32;
        let data_size = frames * bytes_per_frame;
        let file_size = 36 + data_size;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&(sample_rate * bytes_per_frame).to_le_bytes());
        buf.extend_from_slice(&(bytes_per_frame as u16).to_le_bytes());
        buf.extend_from_slice(&16u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        buf.extend(vec![0u8; data_size as usize]);
        buf
    }

    /// Enough header for probe, then a reset so `decode_next` sees `IoError`.
    struct DroppingWav {
        data: std::io::Cursor<Vec<u8>>,
        read_bytes: usize,
        drop_after: usize,
    }

    impl DroppingWav {
        fn new() -> Self {
            Self {
                data: std::io::Cursor::new(silent_wav_bytes(44_100)),
                read_bytes: 0,
                drop_after: 4_096,
            }
        }
    }

    impl std::io::Read for DroppingWav {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.read_bytes >= self.drop_after {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "dropped connection",
                ));
            }
            let n = self.data.read(buf)?;
            self.read_bytes += n;
            Ok(n)
        }
    }

    impl std::io::Seek for DroppingWav {
        fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "no seek",
            ))
        }
    }

    impl symphonia::core::io::MediaSource for DroppingWav {
        fn is_seekable(&self) -> bool {
            false
        }
        fn byte_len(&self) -> Option<u64> {
            None
        }
    }

    struct MockStreamSource;

    impl AudioSource for MockStreamSource {
        fn path(&self) -> Option<&Path> {
            None
        }
        fn url(&self) -> Option<&str> {
            Some("http://127.0.0.1:1/stream")
        }
        fn title_hint(&self) -> &str {
            "Mock"
        }
        fn read_info(&self) -> Result<TrackInfo> {
            unimplemented!()
        }
        fn open_reader(&self) -> Result<Box<dyn symphonia::core::io::MediaSource>> {
            Ok(Box::new(DroppingWav::new()))
        }
    }

    #[test]
    fn stream_io_error_returns_decode_error_not_eof() {
        let (mut decoder, _) = AudioDecoder::open(&MockStreamSource).expect("probe stream wav");
        let mut err = None;
        for _ in 0..32 {
            match decoder.decode_next() {
                Err(e) => {
                    err = Some(e);
                    break;
                }
                Ok(None) => panic!("stream ended silently on io error"),
                Ok(Some(_)) => {}
            }
        }
        let err = err.expect("expected an error from dropping connection");
        assert!(
            err.to_string().contains("dropped connection"),
            "got error: {err}"
        );
    }
}
