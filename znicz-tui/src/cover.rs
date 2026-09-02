use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossbeam_channel::{unbounded, Sender};
use image::{imageops, imageops::FilterType, DynamicImage, ImageBuffer, Rgba};

const MAX_EDGE: u32 = 512;
const CACHE_CAP: usize = 16;
const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");
/// Opaque fill for letterboxing. Transparent pad leaves the previous Kitty
/// image in those cells.
const SLOT_BG: Rgba<u8> = Rgba([0x28, 0x2c, 0x34, 0xff]);

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
}

type Map = Arc<Mutex<(HashMap<PathBuf, Slot>, VecDeque<PathBuf>)>>;

pub struct CoverCache {
    map: Map,
    requests: Sender<PathBuf>,
    logo: Arc<DynamicImage>,
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
        let map: Map = Arc::new(Mutex::new((HashMap::new(), VecDeque::new())));
        let (requests, incoming) = unbounded::<PathBuf>();
        let worker_map = map.clone();
        std::thread::Builder::new()
            .name("znicz-cover".into())
            .spawn(move || {
                while let Ok(path) = incoming.recv() {
                    let ready = match znicz_core::read_cover(&path) {
                        Some(art) => match decode_capped(&art.bytes) {
                            Some(img) => CoverReady::Embedded(Arc::new(img)),
                            None => CoverReady::Logo,
                        },
                        None => CoverReady::Logo,
                    };
                    let mut guard = worker_map.lock().unwrap();
                    let (cache, order) = &mut *guard;
                    cache.insert(path.clone(), Slot { ready });
                    if !order.iter().any(|p| p == &path) {
                        order.push_back(path);
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

    /// `None` path (stream / nothing loaded) is the logo immediately.
    pub fn get(&self, path: Option<&Path>) -> CoverReady {
        let Some(path) = path else {
            return CoverReady::Logo;
        };
        let mut guard = self.map.lock().unwrap();
        let (cache, _) = &mut *guard;
        if let Some(slot) = cache.get(path) {
            return slot.ready.clone();
        }
        cache.insert(
            path.to_path_buf(),
            Slot {
                ready: CoverReady::Pending,
            },
        );
        drop(guard);
        self.requests.send(path.to_path_buf()).ok();
        CoverReady::Pending
    }
}

/// Scale `image` (up or down) to fit inside the cover cells, then paint it
/// onto an opaque canvas the size of the slot. Every cell gets a pixel so a
/// previous graphics-protocol image cannot remain.
pub fn fill_cover_slot(
    image: &DynamicImage,
    cols: u16,
    rows: u16,
    font_w: u16,
    font_h: u16,
) -> DynamicImage {
    let width = (u32::from(cols) * u32::from(font_w.max(1))).max(1);
    let height = (u32::from(rows) * u32::from(font_h.max(1))).max(1);
    let mut canvas: DynamicImage = ImageBuffer::from_pixel(width, height, SLOT_BG).into();
    if image.width() == 0 || image.height() == 0 {
        return canvas;
    }
    let fitted = image.resize(width, height, FilterType::Triangle);
    let x = i64::from(width.saturating_sub(fitted.width()) / 2);
    let y = i64::from(height.saturating_sub(fitted.height()) / 2);
    imageops::overlay(&mut canvas, &fitted, x, y);
    canvas
}

fn decode_capped(bytes: &[u8]) -> Option<DynamicImage> {
    let img = image::load_from_memory(bytes).ok()?;
    Some(img.resize(MAX_EDGE, MAX_EDGE, FilterType::Triangle))
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn no_path_is_the_logo() {
        let cache = CoverCache::new();
        assert_eq!(cache.get(None), CoverReady::Logo);
    }

    #[test]
    fn missing_file_resolves_to_logo() {
        let cache = CoverCache::new();
        let path = PathBuf::from("/definitely/not/a/cover.flac");
        assert_eq!(cache.get(Some(&path)), CoverReady::Pending);
        let mut ready = CoverReady::Pending;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            ready = cache.get(Some(&path));
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
        assert_eq!(cache.get(Some(&path)), CoverReady::Pending);
        let mut ready = CoverReady::Pending;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            ready = cache.get(Some(&path));
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
