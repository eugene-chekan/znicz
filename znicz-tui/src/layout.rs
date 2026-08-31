//! Overlay vs full-width sheet, and the library strip the drawer leaves open.

use ratatui::layout::Rect;

pub const DRAWER_WIDTH: u16 = 41;
pub const MIN_STRIP: u16 = 40;

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

/// Height of one boxed toast, including its own border.
pub const TOAST_BOX_HEIGHT: u16 = 3;
/// Gap from the pane border, so the toast does not sit on the line.
pub const TOAST_INSET: u16 = 1;
/// Mark plus the space after it (`× message`).
pub const TOAST_MARK_AND_SPACE: u16 = 2;
/// Left and right border of a boxed toast.
pub const TOAST_BORDERS: u16 = 2;
/// Narrowest list that still gets a boxed toast.
const TOAST_BOX_MIN_WIDTH: u16 = 12;

/// A 3-row bordered box when the list can hold it; otherwise a 1-row line.
pub fn toast_boxed(list: Rect) -> bool {
    list.height >= TOAST_BOX_HEIGHT + TOAST_INSET * 2 && list.width >= TOAST_BOX_MIN_WIDTH
}

/// Columns needed to draw a toast without cutting `text`.
///
/// `boxed` is the 3-row bordered box used when the list is large enough.
pub fn toast_width_for(text: &str, boxed: bool) -> u16 {
    let borders = if boxed { TOAST_BORDERS as usize } else { 0 };
    text.chars()
        .count()
        .saturating_add(TOAST_MARK_AND_SPACE as usize)
        .saturating_add(borders)
        .max(1) as u16
}

/// Bottom-right boxes for up to three toasts, inset from the pane border.
pub fn toast_areas(list: Rect, count: u16, line_width: u16) -> Vec<Rect> {
    let count = count.min(3);
    if count == 0 || list.width == 0 || list.height == 0 {
        return Vec::new();
    }

    let boxed = toast_boxed(list);
    let inset = if boxed { TOAST_INSET } else { 0 };
    let height = if boxed { TOAST_BOX_HEIGHT } else { 1 };
    let width = line_width
        .min(list.width.saturating_sub(inset.saturating_mul(2)))
        .max(1);
    let x = list.x + list.width.saturating_sub(width).saturating_sub(inset);

    (0..count)
        .filter_map(|i| {
            let stacked = height.saturating_mul(i + 1);
            let y = list
                .y
                .saturating_add(list.height)
                .saturating_sub(inset)
                .saturating_sub(stacked);
            if y < list.y {
                return None;
            }
            Some(Rect {
                x,
                y,
                width,
                height,
            })
        })
        .collect()
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
    fn toast_areas_stack_up_from_the_bottom_right() {
        let list = Rect::new(0, 0, 80, 10);
        let areas = toast_areas(list, 3, 32);
        assert_eq!(areas.len(), 3);
        // Inset 1 from the right and from the bottom; each box is 3 rows.
        assert_eq!(areas[0], Rect::new(47, 6, 32, 3));
        assert_eq!(areas[1], Rect::new(47, 3, 32, 3));
        assert_eq!(areas[2], Rect::new(47, 0, 32, 3));
        assert!(
            areas[0].bottom() < list.bottom(),
            "must not sit on the pane's bottom border"
        );
        assert!(
            areas[0].right() < list.right(),
            "must not sit on the pane's right border"
        );
    }

    #[test]
    fn a_tiny_list_falls_back_to_a_single_line() {
        let list = Rect::new(0, 0, 20, 3);
        let areas = toast_areas(list, 1, 32);
        assert_eq!(areas.len(), 1);
        assert_eq!(areas[0].height, 1);
        assert_eq!(areas[0].y, 2);
    }

    #[test]
    fn toast_width_covers_the_playlist_empty_error() {
        let text = "player error: playlist had no playable files";
        let width = toast_width_for(text, true);
        assert!(
            width > 42,
            "must be wider than the old 40% cap so the reason fits, got {width}"
        );
    }
}
