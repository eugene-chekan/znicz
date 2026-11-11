use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Sender};
use image::{imageops, imageops::FilterType, DynamicImage, ImageBuffer, Rgba};

const MAX_EDGE: u32 = 512;
const CACHE_CAP: usize = 16;
/// How often a stream without ICY titles is looked up again (the AudioAddict
/// channel map lifetime), so its cover follows the playing track. Also the
/// wait before retrying a lookup that came back unsure (a source was down).
const STREAM_REFRESH: Duration = Duration::from_secs(15);
/// The wait before asking again after a `Soon` answer (the source's data is
/// catching up with the stream title).
const SOON_RETRY: Duration = Duration::from_millis(1500);
const LOGO_PNG: &[u8] = include_bytes!("../assets/lantern-icon-1024.png");
/// Opaque fill for letterboxing when the terminal does not report its own
/// background. Transparent pad leaves the previous Kitty image in those cells.
const SLOT_BG: Rgba<u8> = Rgba([0x28, 0x2c, 0x34, 0xff]);

/// The cover slot's fill: the terminal's background colour when it answered
/// the OSC 11 query, else [`SLOT_BG`]. The transparent logo then blends into
/// the terminal.
pub fn slot_background(capabilities: &[ratatui_image::picker::Capability]) -> Rgba<u8> {
    capabilities
        .iter()
        .find_map(|capability| match capability {
            ratatui_image::picker::Capability::Background(r, g, b) => {
                Some(Rgba([*r, *g, *b, 0xff]))
            }
            _ => None,
        })
        .unwrap_or(SLOT_BG)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoverKey {
    /// A local file's embedded picture.
    File(PathBuf),
    /// A radio stream, resolved through the cover source chain.
    Stream(znicz_core::StreamQuery),
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
    resolved_at: Instant,
    /// Retry the lookup this long after the last answer: unsure answers wait
    /// the refresh interval, `Soon` answers a moment. `None` for a settled answer.
    retry_after: Option<Duration>,
}

/// How long to wait before asking a stream's cover again. Read by the worker
/// and by [`CoverCache::get`], so the builders still apply after the worker
/// has been spawned.
#[derive(Debug, Clone, Copy)]
struct Intervals {
    /// Title-less streams, and unsure answers.
    stream_refresh: Duration,
    /// `Soon` answers.
    soon_retry: Duration,
}

type Map = Arc<Mutex<(HashMap<CoverKey, Slot>, VecDeque<CoverKey>)>>;

pub struct CoverCache {
    map: Map,
    requests: Sender<CoverKey>,
    logo: Arc<DynamicImage>,
    intervals: Arc<Mutex<Intervals>>,
}

impl Default for CoverCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverCache {
    pub fn new() -> Self {
        Self::with_streams(znicz_core::StreamCovers::new(znicz_core::default_sources()))
    }

    /// A cache whose streams resolve through `streams`.
    pub fn with_streams(streams: znicz_core::StreamCovers) -> Self {
        let logo = Arc::new(
            image::load_from_memory(LOGO_PNG).unwrap_or_else(|_| DynamicImage::new_rgb8(1, 1)),
        );
        let map: Map = Arc::new(Mutex::new((HashMap::new(), VecDeque::new())));
        let intervals = Arc::new(Mutex::new(Intervals {
            stream_refresh: STREAM_REFRESH,
            soon_retry: SOON_RETRY,
        }));
        let (requests, incoming) = unbounded::<CoverKey>();
        let worker_map = map.clone();
        let worker_intervals = intervals.clone();
        std::thread::Builder::new()
            .name("znicz-cover".into())
            .spawn(move || {
                while let Ok(mut key) = incoming.recv() {
                    // Only the newest queued request is worth resolving now.
                    let mut skipped = Vec::new();
                    while let Ok(newer) = incoming.try_recv() {
                        skipped.push(std::mem::replace(&mut key, newer));
                    }
                    let current = *worker_intervals.lock().unwrap();
                    let result = resolve_cover(&key, &streams, current);
                    let mut guard = worker_map.lock().unwrap();
                    let (cache, order) = &mut *guard;
                    match result {
                        ResolveResult::Update { ready } => {
                            cache.insert(
                                key.clone(),
                                Slot {
                                    ready,
                                    refreshing: false,
                                    resolved_at: Instant::now(),
                                    retry_after: None,
                                },
                            );
                        }
                        ResolveResult::KeepPrevious { retry_after } => {
                            if let Some(slot) = cache.get_mut(&key) {
                                slot.refreshing = false;
                                slot.resolved_at = Instant::now();
                                slot.retry_after = Some(retry_after);
                            }
                        }
                    }
                    for stale in skipped.iter().filter(|k| **k != key) {
                        let pending = matches!(
                            cache.get(stale),
                            Some(slot) if matches!(slot.ready, CoverReady::Pending)
                        );
                        if pending {
                            // Never answered: forget it so the next lookup asks again.
                            cache.remove(stale);
                            order.retain(|k| k != stale);
                        } else if let Some(slot) = cache.get_mut(stale) {
                            slot.refreshing = false;
                        }
                    }
                    if cache.contains_key(&key) && !order.iter().any(|k| k == &key) {
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
            intervals,
        }
    }

    /// Use another refresh interval for title-less streams (tests).
    pub fn with_stream_refresh(self, every: Duration) -> Self {
        self.intervals.lock().unwrap().stream_refresh = every;
        self
    }

    /// Use another retry delay after a `Soon` answer (tests).
    pub fn with_soon_retry(self, every: Duration) -> Self {
        self.intervals.lock().unwrap().soon_retry = every;
        self
    }

    pub fn logo_image(&self) -> &DynamicImage {
        self.logo.as_ref()
    }

    pub fn get(&self, key: CoverKey) -> CoverReady {
        let stream_refresh = self.intervals.lock().unwrap().stream_refresh;
        let mut guard = self.map.lock().unwrap();
        let (cache, _) = &mut *guard;
        if let Some(slot) = cache.get_mut(&key) {
            // A title-less stream follows its track; a pending lookup is never
            // re-sent unless it asked to be retried.
            let untitled_stream = matches!(&key, CoverKey::Stream(query) if query.icy_title.is_none())
                && !matches!(slot.ready, CoverReady::Pending);
            let due = match slot.retry_after {
                Some(after) => slot.resolved_at.elapsed() >= after,
                None => untitled_stream && slot.resolved_at.elapsed() >= stream_refresh,
            };
            let refresh = !slot.refreshing && due;
            if refresh {
                slot.refreshing = true;
                let ready = slot.ready.clone();
                drop(guard);
                self.requests.send(key).ok();
                return ready;
            }
            return slot.ready.clone();
        }
        cache.insert(
            key.clone(),
            Slot {
                ready: CoverReady::Pending,
                refreshing: false,
                resolved_at: Instant::now(),
                retry_after: None,
            },
        );
        drop(guard);
        self.requests.send(key).ok();
        CoverReady::Pending
    }
}

enum ResolveResult {
    Update { ready: CoverReady },
    KeepPrevious { retry_after: Duration },
}

fn resolve_cover(
    key: &CoverKey,
    streams: &znicz_core::StreamCovers,
    intervals: Intervals,
) -> ResolveResult {
    let ready = |bytes: &[u8]| match decode_capped(bytes) {
        Some(img) => CoverReady::Embedded(Arc::new(img)),
        None => CoverReady::Logo,
    };
    match key {
        CoverKey::File(path) => ResolveResult::Update {
            ready: znicz_core::read_cover(path).map_or(CoverReady::Logo, |art| ready(&art.bytes)),
        },
        CoverKey::Stream(query) => match streams.resolve(query) {
            znicz_core::Resolution::Found(art) => ResolveResult::Update {
                ready: ready(&art.bytes),
            },
            znicz_core::Resolution::Nothing => ResolveResult::Update {
                ready: CoverReady::Logo,
            },
            znicz_core::Resolution::Unsure => ResolveResult::KeepPrevious {
                retry_after: intervals.stream_refresh,
            },
            znicz_core::Resolution::Soon => ResolveResult::KeepPrevious {
                retry_after: intervals.soon_retry,
            },
        },
    }
}

/// Whether the cover slot should leave uncovered pixels transparent.
///
/// Sixel stores colours as whole percentages, so no fill can match the
/// terminal background exactly (`#282c34` comes back as `#262b33`). Leaving
/// those pixels undrawn shows the terminal's own colour instead. That is safe
/// because the Sixel renderer erases the cells before drawing. Kitty does not
/// erase, so a transparent pixel would show the previous cover; Kitty and
/// half-blocks keep an opaque fill, which they show exactly.
pub fn slot_is_see_through(protocol: ratatui_image::picker::ProtocolType) -> bool {
    protocol == ratatui_image::picker::ProtocolType::Sixel
}

/// Scale `image` (up or down) to fit inside the cover cells, then paint it
/// onto a `background` canvas the size of the slot. Box-drawing `│` sits in the
/// middle of a cell; graphics fill the cell, so the bitmap is inset by half a
/// cell and pinned to the top.
///
/// The canvas is opaque, so every cell gets a pixel and a previous
/// graphics-protocol image cannot remain. With `see_through`, pixels the image
/// does not cover at all (letterbox and fully transparent art) are left
/// transparent instead; partly covered pixels stay blended onto `background`
/// and opaque, so soft edges do not turn jagged.
pub fn fill_cover_slot(
    image: &DynamicImage,
    cols: u16,
    rows: u16,
    font_w: u16,
    font_h: u16,
    background: Rgba<u8>,
    see_through: bool,
) -> DynamicImage {
    let font_w = font_w.max(1);
    let width = (u32::from(cols) * u32::from(font_w)).max(1);
    let height = (u32::from(rows) * u32::from(font_h.max(1))).max(1);
    let mut canvas = ImageBuffer::from_pixel(width, height, background);
    let pad_x = u32::from(font_w) / 2;
    let fitted = (image.width() > 0 && image.height() > 0)
        .then(|| image.resize(width, height, FilterType::Triangle).to_rgba8());
    if let Some(fitted) = &fitted {
        imageops::overlay(&mut canvas, fitted, i64::from(pad_x), 0);
    }
    // Blending can round alpha down to 254; set it outright.
    for (x, y, pixel) in canvas.enumerate_pixels_mut() {
        let covered = || {
            fitted
                .as_ref()
                .filter(|_| x >= pad_x)
                .and_then(|fitted| fitted.get_pixel_checked(x - pad_x, y))
                .is_some_and(|source| source[3] > 0)
        };
        pixel[3] = if see_through && !covered() { 0 } else { 0xff };
    }
    canvas.into()
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
    fn bundled_logo_has_a_transparent_background() {
        let cache = CoverCache::new();
        let logo = cache.logo_image();
        assert_eq!(logo.get_pixel(0, 0)[3], 0, "corner should be transparent");
        let (w, h) = (logo.width(), logo.height());
        assert!(
            (0..w).any(|x| logo.get_pixel(x, h / 2)[3] == 255),
            "the candle itself stays opaque"
        );
    }

    #[test]
    fn the_slot_takes_the_terminal_background_when_reported() {
        use ratatui_image::picker::Capability;
        let caps = [Capability::Kitty, Capability::Background(10, 20, 30)];
        assert_eq!(slot_background(&caps), Rgba([10, 20, 30, 0xff]));
        assert_eq!(slot_background(&[Capability::Kitty]), SLOT_BG);
    }

    #[test]
    fn transparent_pixels_show_the_slot_background() {
        let src = DynamicImage::new_rgba8(4, 4);
        let bg = Rgba([10, 20, 30, 0xff]);
        let out = fill_cover_slot(&src, 4, 4, 8, 16, bg, false);
        let pad = 8 / 2;
        assert_eq!(out.get_pixel(pad + 1, 1), bg);
    }

    #[test]
    fn only_sixel_slots_are_see_through() {
        use ratatui_image::picker::ProtocolType;
        assert!(slot_is_see_through(ProtocolType::Sixel));
        assert!(!slot_is_see_through(ProtocolType::Kitty));
        assert!(!slot_is_see_through(ProtocolType::Halfblocks));
    }

    #[test]
    fn a_see_through_slot_leaves_uncovered_pixels_transparent() {
        let src = DynamicImage::new_rgba8(4, 4);
        let out = fill_cover_slot(&src, 4, 4, 8, 16, SLOT_BG, true);
        assert_eq!(out.get_pixel(0, 0)[3], 0, "letterbox pad");
        assert_eq!(out.get_pixel(8 / 2 + 1, 1)[3], 0, "transparent source");
    }

    #[test]
    fn a_see_through_slot_blends_soft_edges_onto_the_background() {
        let src: DynamicImage = ImageBuffer::from_pixel(4, 4, Rgba([255u8, 0, 0, 128])).into();
        let bg = Rgba([0, 0, 255, 0xff]);
        let out = fill_cover_slot(&src, 4, 4, 8, 16, bg, true);
        let px = out.get_pixel(8 / 2 + 1, 1);
        assert_eq!(px[3], 0xff, "a partly covered pixel is drawn solid");
        assert!(
            px[0] > 100 && px[2] > 100,
            "red blended onto blue, got {px:?}"
        );
    }

    #[test]
    fn a_see_through_slot_keeps_opaque_art() {
        let src: DynamicImage = ImageBuffer::from_pixel(4, 4, Rgba([255u8, 0, 0, 255])).into();
        let out = fill_cover_slot(&src, 4, 4, 8, 16, SLOT_BG, true);
        assert_eq!(out.get_pixel(8 / 2 + 1, 1), Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn even_a_one_pixel_source_fills_the_cover_slot() {
        let src = DynamicImage::new_rgb8(1, 1);
        let out = fill_cover_slot(&src, 16, 8, 8, 16, SLOT_BG, false);
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
        let out = fill_cover_slot(&src, 16, 8, font_w, 16, SLOT_BG, false);
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

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x67, 0xF0, 0xF7, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    /// Always finds the 1×1 PNG and counts lookups.
    struct Counting(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl znicz_core::CoverSource for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn key(&self, query: &znicz_core::StreamQuery) -> Option<String> {
            Some(query.icy_title.clone().unwrap_or_default())
        }
        fn find(&self, _query: &znicz_core::StreamQuery) -> znicz_core::Found {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            znicz_core::Found::Art(znicz_core::CoverArt {
                mime: "image/png".into(),
                bytes: PNG_1X1.to_vec(),
            })
        }
        fn remembers(&self) -> bool {
            false
        }
    }

    fn stream_query(title: Option<&str>) -> znicz_core::StreamQuery {
        znicz_core::StreamQuery {
            stream_url: "http://radio/stream".into(),
            station_name: "Radio".into(),
            icy_title: title.map(str::to_string),
            icy_url: None,
            station_art: None,
            search: true,
        }
    }

    fn wait_ready(cache: &CoverCache, key: &CoverKey) -> CoverReady {
        let start = std::time::Instant::now();
        loop {
            let ready = cache.get(key.clone());
            if ready != CoverReady::Pending || start.elapsed() > Duration::from_secs(5) {
                return ready;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_stream_key_resolves_through_the_chain() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let streams = znicz_core::StreamCovers::new(vec![Box::new(Counting(calls))]);
        let cache = CoverCache::with_streams(streams);
        let key = CoverKey::Stream(stream_query(Some("A - B")));
        assert!(matches!(wait_ready(&cache, &key), CoverReady::Embedded(_)));
    }

    /// Waits (bounded) until the source has been called `want` times.
    fn wait_for_calls(
        calls: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
        want: usize,
    ) -> usize {
        let start = std::time::Instant::now();
        loop {
            let n = calls.load(std::sync::atomic::Ordering::SeqCst);
            if n >= want || start.elapsed() > Duration::from_secs(5) {
                return n;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn only_streams_without_a_title_are_refreshed() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let streams = znicz_core::StreamCovers::new(vec![Box::new(Counting(calls.clone()))]);
        let cache =
            CoverCache::with_streams(streams).with_stream_refresh(Duration::from_millis(100));
        let titled = CoverKey::Stream(stream_query(Some("A - B")));
        let untitled = CoverKey::Stream(stream_query(None));
        wait_ready(&cache, &titled);
        wait_ready(&cache, &untitled);
        assert_eq!(wait_for_calls(&calls, 2), 2);

        // Poll the untitled stream until its interval has passed and it refreshes.
        let start = std::time::Instant::now();
        while calls.load(std::sync::atomic::Ordering::SeqCst) < 3
            && start.elapsed() < Duration::from_secs(5)
        {
            assert!(
                matches!(cache.get(untitled.clone()), CoverReady::Embedded(_)),
                "a refreshing entry keeps its image"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(wait_for_calls(&calls, 3), 3);

        // The titled stream never re-requests, however long it is polled.
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(250) {
            assert!(matches!(cache.get(titled.clone()), CoverReady::Embedded(_)));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "only the untitled stream was refreshed"
        );
    }

    /// Finds nothing useful on the first lookup (unreachable), art after that.
    struct Flaky(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl znicz_core::CoverSource for Flaky {
        fn name(&self) -> &'static str {
            "flaky"
        }
        fn key(&self, query: &znicz_core::StreamQuery) -> Option<String> {
            Some(query.icy_title.clone().unwrap_or_default())
        }
        fn find(&self, _query: &znicz_core::StreamQuery) -> znicz_core::Found {
            let before = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if before == 0 {
                znicz_core::Found::Unavailable
            } else {
                znicz_core::Found::Art(znicz_core::CoverArt {
                    mime: "image/png".into(),
                    bytes: PNG_1X1.to_vec(),
                })
            }
        }
        fn remembers(&self) -> bool {
            false
        }
    }

    /// Takes `pause` per lookup, then finds the 1×1 PNG.
    struct Slow {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        pause: Duration,
    }

    impl znicz_core::CoverSource for Slow {
        fn name(&self) -> &'static str {
            "slow"
        }
        fn key(&self, query: &znicz_core::StreamQuery) -> Option<String> {
            Some(query.icy_title.clone().unwrap_or_default())
        }
        fn find(&self, _query: &znicz_core::StreamQuery) -> znicz_core::Found {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(self.pause);
            znicz_core::Found::Art(znicz_core::CoverArt {
                mime: "image/png".into(),
                bytes: PNG_1X1.to_vec(),
            })
        }
        fn remembers(&self) -> bool {
            false
        }
    }

    #[test]
    fn an_unsure_answer_is_retried_after_the_interval() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let streams = znicz_core::StreamCovers::new(vec![Box::new(Flaky(calls.clone()))]);
        let cache =
            CoverCache::with_streams(streams).with_stream_refresh(Duration::from_millis(100));
        let key = CoverKey::Stream(stream_query(Some("A - B")));

        assert_eq!(cache.get(key.clone()), CoverReady::Pending);
        assert_eq!(wait_for_calls(&calls, 1), 1);
        assert_eq!(
            cache.get(key.clone()),
            CoverReady::Pending,
            "an unsure answer keeps the slot pending"
        );

        assert!(matches!(wait_ready(&cache, &key), CoverReady::Embedded(_)));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// Answers `Soon` on the first lookup, art after that.
    struct SoonThenArt(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl znicz_core::CoverSource for SoonThenArt {
        fn name(&self) -> &'static str {
            "soon"
        }
        fn key(&self, query: &znicz_core::StreamQuery) -> Option<String> {
            Some(query.icy_title.clone().unwrap_or_default())
        }
        fn find(&self, _query: &znicz_core::StreamQuery) -> znicz_core::Found {
            let before = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if before == 0 {
                znicz_core::Found::Soon
            } else {
                znicz_core::Found::Art(znicz_core::CoverArt {
                    mime: "image/png".into(),
                    bytes: PNG_1X1.to_vec(),
                })
            }
        }
        fn remembers(&self) -> bool {
            false
        }
    }

    #[test]
    fn a_soon_answer_is_retried_after_the_short_interval() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let streams = znicz_core::StreamCovers::new(vec![Box::new(SoonThenArt(calls.clone()))]);
        // The refresh interval is far longer than the test, so only the short
        // "soon" retry can bring the art in time.
        let cache = CoverCache::with_streams(streams)
            .with_stream_refresh(Duration::from_secs(60))
            .with_soon_retry(Duration::from_millis(100));
        let key = CoverKey::Stream(stream_query(Some("A - B")));

        assert!(matches!(wait_ready(&cache, &key), CoverReady::Embedded(_)));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn a_pending_stream_is_not_requested_twice() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slow = Slow {
            calls: calls.clone(),
            pause: Duration::from_millis(300),
        };
        let streams = znicz_core::StreamCovers::new(vec![Box::new(slow)]);
        let cache =
            CoverCache::with_streams(streams).with_stream_refresh(Duration::from_millis(50));
        let key = CoverKey::Stream(stream_query(None));

        assert_eq!(cache.get(key.clone()), CoverReady::Pending);
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(250) {
            cache.get(key.clone());
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(matches!(wait_ready(&cache, &key), CoverReady::Embedded(_)));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Holds its first lookup until the test lets it go, so a test can queue
    /// requests while the worker is known to be busy.
    struct Gated {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        started: std::sync::mpsc::Sender<()>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl znicz_core::CoverSource for Gated {
        fn name(&self) -> &'static str {
            "gated"
        }
        fn key(&self, query: &znicz_core::StreamQuery) -> Option<String> {
            Some(query.icy_title.clone().unwrap_or_default())
        }
        fn find(&self, _query: &znicz_core::StreamQuery) -> znicz_core::Found {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                self.started.send(()).ok();
                let release = self.release.lock().unwrap();
                release.recv_timeout(Duration::from_secs(5)).ok();
            }
            znicz_core::Found::Art(znicz_core::CoverArt {
                mime: "image/png".into(),
                bytes: PNG_1X1.to_vec(),
            })
        }
        fn remembers(&self) -> bool {
            false
        }
    }

    #[test]
    fn only_the_newest_queued_request_is_resolved() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (started_tx, started) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let gated = Gated {
            calls: calls.clone(),
            started: started_tx,
            release: std::sync::Mutex::new(release_rx),
        };
        let cache = CoverCache::with_streams(znicz_core::StreamCovers::new(vec![Box::new(gated)]));
        let key = |title: &str| CoverKey::Stream(stream_query(Some(title)));
        let (a, b, c, d) = (key("a"), key("b"), key("c"), key("d"));

        // A is being looked up; B, C and D queue up behind it.
        assert_eq!(cache.get(a), CoverReady::Pending);
        started
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker starts on A");
        assert_eq!(cache.get(b.clone()), CoverReady::Pending);
        assert_eq!(cache.get(c), CoverReady::Pending);
        assert_eq!(cache.get(d.clone()), CoverReady::Pending);
        release.send(()).unwrap();

        assert!(matches!(wait_ready(&cache, &d), CoverReady::Embedded(_)));
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "A and D only; B and C were superseded"
        );

        assert_eq!(
            cache.get(b.clone()),
            CoverReady::Pending,
            "a skipped request is requested again on the next lookup"
        );
        assert!(matches!(wait_ready(&cache, &b), CoverReady::Embedded(_)));
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.flac");
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
