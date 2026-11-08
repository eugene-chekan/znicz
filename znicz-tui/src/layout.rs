//! Overlay vs full-width sheet, and the library strip the drawer leaves open.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub const DRAWER_WIDTH: u16 = 41;
pub const MIN_STRIP: u16 = 40;
/// Minimum library strip inner width (characters) for three-column browse.
pub const MIN_COLUMNS_STRIP: usize = 60;

/// Whether the library strip is wide enough for three columns.
pub fn columns_usable(strip_inner: usize) -> bool {
    strip_inner >= MIN_COLUMNS_STRIP
}

/// Below this height the signal-path line is dropped to keep the lists usable.
pub const COMPACT_HEIGHT: u16 = 20;
/// The cover only shows on a window at least this tall; shorter ones keep the list.
pub const COVER_MIN_WINDOW: u16 = 24;
const COVER_ROWS: u16 = 8;
/// Below this list-region height a modal takes the whole list region.
pub const MODAL_FULL_BELOW: u16 = 10;

pub fn cover_enabled(window: u16, show_cover: bool) -> bool {
    show_cover && window >= COVER_MIN_WINDOW
}

/// Transport rows, including the message row.
pub fn transport_height(window: u16, show_cover: bool) -> u16 {
    if cover_enabled(window, show_cover) {
        COVER_ROWS
    } else if window < COMPACT_HEIGHT {
        2
    } else {
        3
    }
}

/// A `want_w` × `want_h` box centred in the list region and clamped to it.
///
/// A short list region gives the modal all of it.
pub fn modal_rect(list: Rect, want_w: u16, want_h: u16) -> Rect {
    if list.height < MODAL_FULL_BELOW {
        return list;
    }
    let width = want_w.min(list.width);
    let height = want_h.min(list.height);
    Rect {
        x: list.x + (list.width - width) / 2,
        y: list.y + (list.height - height) / 2,
        width,
        height,
    }
}

/// Devices, Playlists, Radio, Messages: 70 % of the list region, at least 40 × 8.
pub fn list_modal_rect(list: Rect) -> Rect {
    let width = ((u32::from(list.width) * 70 / 100) as u16).max(40);
    let height = ((u32::from(list.height) * 70 / 100) as u16).max(8);
    modal_rect(list, width, height)
}

pub fn cover_width(cover_rows: u16, font_w: u16, font_h: u16, area_width: u16) -> u16 {
    if cover_rows == 0 || font_w == 0 || area_width == 0 {
        return 0;
    }
    let cols = (u32::from(cover_rows) * u32::from(font_h) / u32::from(font_w)).max(1) as u16;
    cols.min(area_width / 2).max(1)
}

pub fn cover_chrome_split(area: Rect, cover_w: u16) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(cover_w),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(area);
    (chunks[0], chunks[2])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawer {
    Closed,
    Overlay(Rect),
    Sheet(Rect),
}

pub fn drawer(list: Rect, open: bool) -> Drawer {
    if !open {
        return Drawer::Closed;
    }
    if list.width <= DRAWER_WIDTH.saturating_add(MIN_STRIP) {
        Drawer::Sheet(list)
    } else {
        Drawer::Overlay(Rect {
            x: list.x + list.width - DRAWER_WIDTH,
            y: list.y,
            width: DRAWER_WIDTH,
            height: list.height,
        })
    }
}

pub fn strip_width(list: Rect, open: bool) -> u16 {
    match drawer(list, open) {
        Drawer::Closed | Drawer::Sheet(_) => list.width,
        Drawer::Overlay(rect) => list.width.saturating_sub(rect.width),
    }
}

pub fn strip_inner(list: Rect, open: bool) -> usize {
    match drawer(list, open) {
        Drawer::Overlay(_) => strip_width(list, true).saturating_sub(1) as usize,
        _ => list.width.saturating_sub(2) as usize,
    }
}

pub fn is_sheet(list_width: u16, open: bool) -> bool {
    open && list_width <= DRAWER_WIDTH.saturating_add(MIN_STRIP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(width: u16) -> Rect {
        Rect::new(0, 0, width, 20)
    }

    #[test]
    fn a_wide_list_gets_a_41_column_overlay_on_the_right() {
        let list = list(100);
        match drawer(list, true) {
            Drawer::Overlay(rect) => {
                assert_eq!(rect.width, 41);
                assert_eq!(rect.x, 59);
                assert_eq!(rect.height, 20);
            }
            other => panic!("expected overlay, got {other:?}"),
        }
        assert_eq!(strip_width(list, true), 59);
    }

    #[test]
    fn a_narrow_list_becomes_a_full_width_sheet() {
        // 41 + 40 = 81; at 81 or below the strip would be too thin.
        let list = list(81);
        match drawer(list, true) {
            Drawer::Sheet(rect) => assert_eq!(rect, list),
            other => panic!("expected sheet, got {other:?}"),
        }
        assert_eq!(strip_width(list, true), 81);
        assert!(is_sheet(81, true));
        assert!(!is_sheet(82, true));
    }

    #[test]
    fn a_closed_drawer_leaves_the_full_list() {
        let list = list(80);
        assert_eq!(drawer(list, false), Drawer::Closed);
        assert_eq!(strip_width(list, false), 80);
        assert!(!is_sheet(40, false));
    }

    #[test]
    fn strip_inner_counts_characters_inside_the_visible_library() {
        let wide = list(100);
        // overlay: visible 59, minus left border → 58
        assert_eq!(strip_inner(wide, true), 58);
        // closed: inner_width = 80 - 2 for a 80-wide pane; here 100 - 2
        assert_eq!(strip_inner(wide, false), 98);
    }

    #[test]
    fn cover_uses_eight_rows_on_a_normal_window() {
        assert_eq!(transport_height(24, true), 8);
        assert!(cover_enabled(24, true));
    }

    #[test]
    fn a_modal_is_centred_and_clamped_to_the_list_region() {
        let list = Rect::new(0, 0, 100, 30);
        assert_eq!(modal_rect(list, 40, 10), Rect::new(30, 10, 40, 10));
        assert_eq!(
            modal_rect(list, 200, 50),
            list,
            "never larger than the list"
        );
    }

    #[test]
    fn a_short_list_region_gives_the_modal_all_of_it() {
        let list = Rect::new(0, 2, 80, 9);
        assert_eq!(modal_rect(list, 40, 5), list);
    }

    #[test]
    fn list_modals_take_seventy_percent_with_a_floor() {
        let list = Rect::new(0, 0, 100, 30);
        assert_eq!(list_modal_rect(list), Rect::new(15, 4, 70, 21));
        let small = Rect::new(0, 0, 50, 10);
        let rect = list_modal_rect(small);
        assert_eq!((rect.width, rect.height), (40, 8));
    }

    #[test]
    fn the_cover_needs_a_24_row_window() {
        assert!(cover_enabled(24, true));
        assert!(!cover_enabled(23, true));
        assert!(!cover_enabled(40, false));
    }

    #[test]
    fn transport_rows_include_the_message_row() {
        assert_eq!(transport_height(30, true), 8);
        assert_eq!(transport_height(22, true), 3, "no cover below 24 rows");
        assert_eq!(transport_height(15, true), 2);
        assert_eq!(transport_height(30, false), 3);
        assert_eq!(transport_height(15, false), 2);
    }

    #[test]
    fn square_cover_columns_follow_cell_aspect() {
        // 8×16 cells, 8 rows → 16 columns; never more than half the row.
        assert_eq!(cover_width(8, 8, 16, 80), 16);
        assert_eq!(cover_width(8, 8, 16, 20), 10);
    }

    #[test]
    fn cover_starts_on_the_library_left_border() {
        let area = Rect::new(0, 0, 80, 8);
        let (cover, chrome) = cover_chrome_split(area, 16);
        assert_eq!(cover.x, 0, "same column as the library pane's left border");
        assert_eq!(cover.width, 16);
        assert_eq!(
            chrome.x,
            cover.x + cover.width + 1,
            "at least one empty cell between the cover and the playback text"
        );
        assert_eq!(cover.y, 0);
    }
}
