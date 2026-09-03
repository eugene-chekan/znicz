use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossbeam_channel::{unbounded, Sender};
use image::{imageops, imageops::FilterType, DynamicImage, ImageBuffer, Rgba};

const MAX_EDGE: u32 = 512;
const CACHE_CAP: usize = 16;
const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");
/// Opaque fill for letterboxing. Transparent pad leaves the previous Kitty
/// image in those cells.
const SLOT_BG: Rgba<u8> = Rgba([0x28, 0x2c, 0x34, 0xff]);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoverKey {
    File(PathBuf),
    ImageFile(PathBuf),
    Url(String),
    AudioAddict {
        network: znicz_core::AudioAddictNetwork,
        channel: String,
    },
}

#[derive(Debug, Clone)]
pub enum CoverReady {
    Pending,
    Logo,
    Embedded(Arc<DynamicImage>),
}

impl PartialEq for CoverReady {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Pending, Self::Pending) | (Self::Logo, Self::Logo) => true,
            (Self::Embedded(a), Self::Embedded(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

struct Slot {
    ready: CoverReady,
    refreshing: bool,
}

type Map = Arc<Mutex<(HashMap<CoverKey, Slot>, VecDeque<CoverKey>, HashSet<String>)>>;

pub struct CoverCache {
    map: Map,
    requests: Sender<CoverKey>,
    logo: Arc<DynamicImage>,
}

pub fn pick_stream_cover(
    icy: CoverReady,
    audioaddict: CoverReady,
    station: CoverReady,
) -> CoverReady {
    if let CoverReady::Embedded(_) = &icy {
        return icy;
    }
    if let CoverReady::Embedded(_) = &audioaddict {
        return audioaddict;
    }
    if let CoverReady::Embedded(_) = &station {
        return station;
    }
    CoverReady::Logo
}

impl Default for CoverCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverCache {
    pub fn new() -> Self {
        let logo = Arc::new(
            image::load_from_memory(LOGO_PNG).unwrap_or_else(|_| DynamicImage::new_rgb8(1, 1)),
        );
        let map: Map = Arc::new(Mutex::new((
            HashMap::new(),
            VecDeque::new(),
            HashSet::new(),
        )));
        let (requests, incoming) = unbounded::<CoverKey>();
        let worker_map = map.clone();
        std::thread::Builder::new()
            .name("znicz-cover".into())
            .spawn(move || {
                while let Ok(key) = incoming.recv() {
                    let result = resolve_cover(&key);
                    let mut guard = worker_map.lock().unwrap();
                    let (cache, order, failed) = &mut *guard;
                    match result {
                        ResolveResult::Update { ready, failed_url } => {
                            if let Some(url) = failed_url {
                                failed.insert(url);
                            }
                            cache.insert(
                                key.clone(),
                                Slot {
                                    ready,
                                    refreshing: false,
                                },
                            );
                        }
                        ResolveResult::KeepPrevious => {
                            if let Some(slot) = cache.get_mut(&key) {
                                slot.refreshing = false;
                            }
                        }
                    }
                    if !order.iter().any(|k| k == &key) {
                        order.push_back(key);
                    }
                    while order.len() > CACHE_CAP {
                        if let Some(old) = order.pop_front() {
                            cache.remove(&old);
                        }
                    }
                }
            })
            .expect("failed to spawn cover reader thread");
        Self {
            map,
            requests,
            logo,
        }
    }

    pub fn logo_image(&self) -> &DynamicImage {
        self.logo.as_ref()
    }

    pub fn get(&self, key: CoverKey) -> CoverReady {
        let audioaddict_stale = matches!(&key, CoverKey::AudioAddict { network, .. } if !znicz_core::audioaddict_cache_fresh(*network));
        let mut guard = self.map.lock().unwrap();
        let (cache, _, failed) = &mut *guard;
        if let CoverKey::Url(url) = &key {
            if failed.contains(url) {
                return CoverReady::Logo;
            }
        }
        if let Some(slot) = cache.get_mut(&key) {
            if let CoverKey::AudioAddict { .. } = &key {
                if audioaddict_stale && !slot.refreshing {
                    slot.refreshing = true;
                    let ready = slot.ready.clone();
                    let key = key.clone();
                    drop(guard);
                    self.requests.send(key).ok();
                    return ready;
                }
            }
            return slot.ready.clone();
        }
        cache.insert(
            key.clone(),
            Slot {
                ready: CoverReady::Pending,
                refreshing: false,
            },
        );
        drop(guard);
        self.requests.send(key).ok();
        CoverReady::Pending
    }
}

enum ResolveResult {
    Update {
        ready: CoverReady,
        failed_url: Option<String>,
    },
    KeepPrevious,
}

fn resolve_cover(key: &CoverKey) -> ResolveResult {
    match key {
        CoverKey::File(path) => ResolveResult::Update {
            ready: match znicz_core::read_cover(path) {
                Some(art) => match decode_capped(&art.bytes) {
                    Some(img) => CoverReady::Embedded(Arc::new(img)),
                    None => CoverReady::Logo,
                },
                None => CoverReady::Logo,
            },
            failed_url: None,
        },
        CoverKey::ImageFile(path) => ResolveResult::Update {
            ready: match std::fs::read(path) {
                Ok(bytes) => match decode_capped(&bytes) {
                    Some(img) => CoverReady::Embedded(Arc::new(img)),
                    None => {
                        tracing::debug!(path = %path.display(), "station art did not decode");
                        CoverReady::Logo
                    }
                },
                Err(e) => {
                    tracing::debug!(path = %path.display(), error = %e, "station art unreadable");
                    CoverReady::Logo
                }
            },
            failed_url: None,
        },
        CoverKey::Url(url) => match znicz_core::fetch_cover(url) {
            Some(art) => match decode_capped(&art.bytes) {
                Some(img) => ResolveResult::Update {
                    ready: CoverReady::Embedded(Arc::new(img)),
                    failed_url: None,
                },
                None => {
                    tracing::debug!(url, "ICY cover URL did not decode as an image");
                    ResolveResult::Update {
                        ready: CoverReady::Logo,
                        failed_url: Some(url.clone()),
                    }
                }
            },
            None => ResolveResult::Update {
                ready: CoverReady::Logo,
                failed_url: Some(url.clone()),
            },
        },
        CoverKey::AudioAddict { network, channel } => {
            match znicz_core::audioaddict_cover_lookup(*network, channel) {
                znicz_core::AudioAddictLookup::Found(url) => match znicz_core::fetch_cover(&url) {
                    Some(art) => match decode_capped(&art.bytes) {
                        Some(img) => ResolveResult::Update {
                            ready: CoverReady::Embedded(Arc::new(img)),
                            failed_url: None,
                        },
                        None => {
                            tracing::debug!(url, "AudioAddict cover did not decode");
                            ResolveResult::Update {
                                ready: CoverReady::Logo,
                                failed_url: Some(url),
                            }
                        }
                    },
                    None => ResolveResult::Update {
                        ready: CoverReady::Logo,
                        failed_url: Some(url),
                    },
                },
                znicz_core::AudioAddictLookup::NoArt => ResolveResult::Update {
                    ready: CoverReady::Logo,
                    failed_url: None,
                },
                znicz_core::AudioAddictLookup::RefreshFailed => ResolveResult::KeepPrevious,
            }
        }
    }
}

/// Scale `image` (up or down) to fit inside the cover cells, then paint it
/// onto an opaque canvas the size of the slot. Box-drawing `│` sits in the
/// middle of a cell; graphics fill the cell, so the bitmap is inset by half a
/// cell and pinned to the top. Every cell gets a pixel so a previous
/// graphics-protocol image cannot remain.
pub fn fill_cover_slot(
    image: &DynamicImage,
    cols: u16,
    rows: u16,
    font_w: u16,
    font_h: u16,
) -> DynamicImage {
    let font_w = font_w.max(1);
    let width = (u32::from(cols) * u32::from(font_w)).max(1);
    let height = (u32::from(rows) * u32::from(font_h.max(1))).max(1);
    let mut canvas: DynamicImage = ImageBuffer::from_pixel(width, height, SLOT_BG).into();
    if image.width() == 0 || image.height() == 0 {
        return canvas;
    }
    let pad_x = u32::from(font_w) / 2;
    let fitted = image.resize(width, height, FilterType::Triangle);
    imageops::overlay(&mut canvas, &fitted, i64::from(pad_x), 0);
    canvas
}

fn decode_capped(bytes: &[u8]) -> Option<DynamicImage> {
    let img = image::load_from_memory(bytes).ok()?;
    Some(img.resize(MAX_EDGE, MAX_EDGE, FilterType::Triangle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn bundled_logo_is_large_enough_to_fill_the_slot() {
        let cache = CoverCache::new();
        let logo = cache.logo_image();
        assert!(
            logo.width() >= 64 && logo.height() >= 64,
            "a tiny placeholder cannot overwrite the previous cover, got {}x{}",
            logo.width(),
            logo.height()
        );
    }

    #[test]
    fn even_a_one_pixel_source_fills_the_cover_slot() {
        let src = DynamicImage::new_rgb8(1, 1);
        let out = fill_cover_slot(&src, 16, 8, 8, 16);
        assert_eq!(out.width(), 16 * 8);
        assert_eq!(out.height(), 8 * 16);
    }

    #[test]
    fn a_wide_source_is_pinned_to_the_top() {
        let mut src = DynamicImage::new_rgb8(100, 50);
        if let DynamicImage::ImageRgb8(ref mut buf) = src {
            for p in buf.pixels_mut() {
                *p = image::Rgb([255, 0, 0]);
            }
        }
        let font_w = 8;
        let out = fill_cover_slot(&src, 16, 8, font_w, 16);
        let pad = u32::from(font_w) / 2;
        let left = out.get_pixel(0, 0);
        assert_eq!(
            [left[0], left[1], left[2]],
            [SLOT_BG[0], SLOT_BG[1], SLOT_BG[2]],
            "half a cell of pad matches the library │, which does not sit on the cell's left edge"
        );
        let art = out.get_pixel(pad, 0);
        assert_eq!([art[0], art[1], art[2]], [255, 0, 0]);
        let bottom = out.get_pixel(pad, out.height() - 1);
        assert_eq!(
            [bottom[0], bottom[1], bottom[2]],
            [SLOT_BG[0], SLOT_BG[1], SLOT_BG[2]]
        );
    }

    #[test]
    fn no_path_is_the_logo() {
        assert_eq!(
            pick_stream_cover(CoverReady::Logo, CoverReady::Logo, CoverReady::Logo),
            CoverReady::Logo
        );
    }

    #[test]
    fn pick_stream_cover_prefers_icy_then_audioaddict_then_station() {
        let icy_img = Arc::new(DynamicImage::new_rgb8(2, 2));
        let aa_img = Arc::new(DynamicImage::new_rgb8(4, 4));
        let station_img = Arc::new(DynamicImage::new_rgb8(3, 3));
        let icy = CoverReady::Embedded(icy_img.clone());
        let aa = CoverReady::Embedded(aa_img.clone());
        let station = CoverReady::Embedded(station_img.clone());
        assert!(matches!(
            pick_stream_cover(icy.clone(), aa.clone(), station.clone()),
            CoverReady::Embedded(img) if Arc::ptr_eq(&img, &icy_img)
        ));
        assert!(matches!(
            pick_stream_cover(CoverReady::Pending, aa.clone(), station.clone()),
            CoverReady::Embedded(img) if Arc::ptr_eq(&img, &aa_img)
        ));
        assert!(matches!(
            pick_stream_cover(CoverReady::Logo, CoverReady::Pending, station.clone()),
            CoverReady::Embedded(img) if Arc::ptr_eq(&img, &station_img)
        ));
        assert!(matches!(
            pick_stream_cover(CoverReady::Logo, CoverReady::Logo, station),
            CoverReady::Embedded(img) if Arc::ptr_eq(&img, &station_img)
        ));
        assert_eq!(
            pick_stream_cover(CoverReady::Pending, CoverReady::Pending, CoverReady::Logo),
            CoverReady::Logo
        );
    }

    fn serve_counting_html(
        hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::Ordering;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                hits.fetch_add(1, Ordering::SeqCst);
                let mut stream = stream;
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = b"<html>nope</html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        (format!("http://{addr}/"), handle)
    }

    #[test]
    fn a_failed_url_is_not_fetched_again() {
        use std::sync::atomic::Ordering;

        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (url, _handle) = serve_counting_html(hits.clone());
        let cache = CoverCache::new();
        let key = CoverKey::Url(url);
        let mut ready = CoverReady::Pending;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            ready = cache.get(key.clone());
            if ready != CoverReady::Pending {
                break;
            }
        }
        assert_eq!(ready, CoverReady::Logo);
        let after_first = hits.load(Ordering::SeqCst);
        assert!(after_first >= 1);
        let _ = cache.get(key);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(hits.load(Ordering::SeqCst), after_first);
    }

    #[test]
    fn missing_file_resolves_to_logo() {
        let cache = CoverCache::new();
        let path = PathBuf::from("/definitely/not/a/cover.flac");
        assert_eq!(cache.get(CoverKey::File(path.clone())), CoverReady::Pending);
        let mut ready = CoverReady::Pending;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            ready = cache.get(CoverKey::File(path.clone()));
            if ready != CoverReady::Pending {
                break;
            }
        }
        assert_eq!(ready, CoverReady::Logo);
    }

    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x67, 0xF0, 0xF7, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    fn write_silent_flac(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let status = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-ac", "1", "-ar", "44100", "-c:a", "flac"])
            .arg(path)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "ffmpeg failed for {}", path.display());
    }

    fn picture(pic_type: lofty::picture::PictureType) -> lofty::picture::Picture {
        use lofty::picture::{MimeType, Picture};
        Picture::unchecked(TINY_PNG.to_vec())
            .pic_type(pic_type)
            .mime_type(MimeType::Png)
            .build()
    }

    fn save_pictures(path: &Path, pics: Vec<lofty::picture::Picture>) {
        use lofty::config::WriteOptions;
        use lofty::prelude::*;
        use lofty::probe::Probe;
        use lofty::tag::Tag;

        let mut tagged = Probe::open(path).unwrap().read().unwrap();
        if tagged.primary_tag().is_none() {
            let tag_type = tagged.primary_tag_type();
            tagged.insert_tag(Tag::new(tag_type));
        }
        let tag = tagged.primary_tag_mut().expect("tag");
        for pic in pics {
            tag.push_picture(pic);
        }
        tag.save_to_path(path, WriteOptions::default())
            .expect("write pictures");
    }

    #[test]
    fn embedded_cover_is_decoded() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not available, skipping");
            return;
        }
        let dir = std::env::temp_dir().join("znicz-cover-cache");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("front.flac");
        write_silent_flac(&path);
        save_pictures(
            &path,
            vec![picture(lofty::picture::PictureType::CoverFront)],
        );

        let cache = CoverCache::new();
        assert_eq!(cache.get(CoverKey::File(path.clone())), CoverReady::Pending);
        let mut ready = CoverReady::Pending;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            ready = cache.get(CoverKey::File(path.clone()));
            if ready != CoverReady::Pending {
                break;
            }
        }
        assert!(
            matches!(ready, CoverReady::Embedded(_)),
            "expected embedded cover, got {ready:?}"
        );
    }
}
