//! Overlay vs full-width sheet, and the library strip the drawer leaves open.

use ratatui::layout::Rect;

pub const DRAWER_WIDTH: u16 = 36;
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

/// Bottom-right boxes for up to three toast lines inside `list`.
pub fn toast_areas(list: Rect, count: u16, line_width: u16) -> Vec<Rect> {
    let count = count.min(3);
    if count == 0 || list.width == 0 || list.height == 0 {
        return Vec::new();
    }
    let width = line_width.min(list.width).max(1);
    let x = list.x + list.width.saturating_sub(width);
    (0..count)
        .map(|i| {
            let y = list.y + list.height.saturating_sub(1).saturating_sub(i);
            Rect {
                x,
                y,
                width,
                height: 1,
            }
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
    fn a_wide_list_gets_a_36_column_overlay_on_the_right() {
        let list = list(100);
        match drawer(list, true) {
            Drawer::Overlay(rect) => {
                assert_eq!(rect.width, 36);
                assert_eq!(rect.x, 64);
                assert_eq!(rect.height, 20);
            }
            other => panic!("expected overlay, got {other:?}"),
        }
        assert_eq!(strip_width(list, true), 64);
    }

    #[test]
    fn a_narrow_list_becomes_a_full_width_sheet() {
        // 36 + 40 = 76; at 76 or below the strip would be too thin.
        let list = list(76);
        match drawer(list, true) {
            Drawer::Sheet(rect) => assert_eq!(rect, list),
            other => panic!("expected sheet, got {other:?}"),
        }
        assert_eq!(strip_width(list, true), 76);
        assert!(is_sheet(76, true));
        assert!(!is_sheet(77, true));
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
        // overlay: visible 64, minus left border → 63
        assert_eq!(strip_inner(wide, true), 63);
        // closed: inner_width = 80 - 2 for a 80-wide pane; here 100 - 2
        assert_eq!(strip_inner(wide, false), 98);
    }

    #[test]
    fn toast_areas_stack_up_from_the_bottom_right() {
        let list = Rect::new(0, 0, 80, 10);
        let areas = toast_areas(list, 3, 32);
        assert_eq!(areas.len(), 3);
        assert_eq!(areas[0], Rect::new(48, 9, 32, 1));
        assert_eq!(areas[1], Rect::new(48, 8, 32, 1));
        assert_eq!(areas[2], Rect::new(48, 7, 32, 1));
    }
}
