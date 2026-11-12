//! The help overlay: this context's keys first, then the rest, with a filter.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;
use crate::help_state::HelpState;
use crate::keys::{self, Section};
use crate::theme;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_PREFIX: &str = " znicz ";
const CLOSE_HINT: &str = " / filter · Esc close ";
const FILTER_HINT: &str = " Enter keep · Esc clear ";

/// Bottom-right border text: what Enter and Esc do right now.
fn border_hint(editing: bool) -> &'static str {
    if editing { FILTER_HINT } else { CLOSE_HINT }
}
const FILTER_PREFIX: &str = "/ ";
/// One column of keys and actions reads best kept narrow.
const MAX_WIDTH: u16 = 64;
/// Fits the longest key label (`Shift-→ / Shift-←`) plus a gap.
const KEY_COLUMN: usize = 19;

pub fn render(frame: &mut Frame, list: Rect, app: &mut App) {
    let popup = crate::layout::modal_rect(list, MAX_WIDTH, list.height);
    app.hits.overlay = Some(popup);
    app.hits.close = crate::views::close_button_rect(popup);

    let hint = border_hint(app.help.editing);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_active())
        .title(Span::styled(title(&app.help), theme::title()))
        .title(crate::views::close_title())
        .title_bottom(Line::from(Span::styled(hint, theme::dim())).right_aligned());
    let update = app.update.as_ref().map(|release| release.version.as_str());
    if let Some(version) = version_title(popup.width, hint, update) {
        block = block.title_bottom(version);
    }
    let inner = block.inner(popup);

    // Clear first, or the pane underneath shows through the overlay.
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let sections = keys::filter_sections(keys::help_sections(app.help.context), &app.help.query);
    let lines = help_lines(&sections);

    let (list_area, prompt_area) = split_prompt(inner, app.help.shows_prompt());
    app.help
        .clamp_scroll(lines.len(), usize::from(list_area.height));
    let scroll = u16::try_from(app.help.scroll).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), list_area);

    if let Some(prompt_area) = prompt_area {
        let line = if app.help.editing {
            crate::views::prompt_line(FILTER_PREFIX, &app.help.query)
        } else {
            crate::views::prompt_line_idle(FILTER_PREFIX, app.help.query.as_str())
        };
        frame.render_widget(Paragraph::new(line), prompt_area);
    }
}

fn title(help: &HelpState) -> String {
    match keys::context_title(help.context) {
        Some(name) => format!(" Keys · {name} "),
        None => " Keys ".to_string(),
    }
}

/// Section titles and binding rows. Sections for other contexts draw dim.
fn help_lines(sections: &[Section]) -> Vec<Line<'static>> {
    if sections.is_empty() {
        return vec![Line::from(Span::styled(" no keys match", theme::dim()))];
    }
    let mut lines = Vec::new();
    for section in sections {
        let (title_style, key_style, action_style) = if section.dim {
            (theme::dim(), theme::dim(), theme::dim())
        } else {
            (theme::strong(), theme::key(), theme::text())
        };
        lines.push(Line::from(Span::styled(section.title, title_style)));
        for binding in &section.bindings {
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("{:<width$}", binding.keys, width = KEY_COLUMN),
                    key_style,
                ),
                Span::styled(binding.action, action_style),
            ]));
        }
    }
    lines
}

/// The list area, and the filter row under it when the filter is showing.
fn split_prompt(inner: Rect, show_prompt: bool) -> (Rect, Option<Rect>) {
    if !show_prompt || inner.height < 2 {
        return (inner, None);
    }
    let list = Rect {
        height: inner.height - 1,
        ..inner
    };
    let prompt = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    (list, Some(prompt))
}

/// The bottom-left version title, or `None` when both titles cannot fit.
///
/// With a newer release known, the title shows the arrow to it.
///
/// The close hint is functional, so it keeps the row when a tiny popup has no
/// room for the version.
fn version_title(popup_width: u16, hint: &str, update: Option<&str>) -> Option<Line<'static>> {
    let label = match update {
        Some(version) => format!("{VERSION_PREFIX}{VERSION} → {version} "),
        None => format!("{VERSION_PREFIX}{VERSION} "),
    };
    let budget = popup_width.saturating_sub(2) as usize;
    if label.chars().count() + hint.chars().count() > budget {
        return None;
    }
    Some(Line::from(Span::styled(label, theme::dim())).left_aligned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Context;

    #[test]
    fn the_title_names_the_context() {
        assert_eq!(title(&HelpState::open(Context::Queue)), " Keys · Queue ");
        assert_eq!(title(&HelpState::open(Context::Prompt)), " Keys ");
    }

    #[test]
    fn every_binding_row_fits_the_popup() {
        use crate::keys::Context::*;
        let contexts = [
            Library,
            LibraryTree,
            Search,
            Queue,
            Devices,
            Settings,
            Stage,
            Playlists,
            Radio,
            Prompt,
            StationPrompt,
            Help,
        ];
        for context in contexts {
            for section in keys::help_sections(context) {
                for binding in section.bindings {
                    let width = 1
                        + KEY_COLUMN.max(binding.keys.chars().count())
                        + binding.action.chars().count();
                    assert!(
                        width <= usize::from(MAX_WIDTH) - 2,
                        "{:?} / {}: {width} columns",
                        binding.keys,
                        binding.action
                    );
                }
            }
        }
    }

    #[test]
    fn every_section_and_binding_gets_a_row() {
        let sections = keys::help_sections(Context::Queue);
        let rows = sections.len() + sections.iter().map(|s| s.bindings.len()).sum::<usize>();
        assert_eq!(help_lines(&sections).len(), rows);
    }

    #[test]
    fn no_sections_draw_one_explaining_row() {
        let lines = help_lines(&[]);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn the_filter_row_takes_the_last_inner_row() {
        let inner = Rect::new(1, 1, 40, 10);
        let (list, prompt) = split_prompt(inner, true);
        assert_eq!(list.height, 9);
        assert_eq!(prompt, Some(Rect::new(1, 10, 40, 1)));
        assert_eq!(split_prompt(inner, false), (inner, None));
    }

    #[test]
    fn the_version_title_names_the_package_version() {
        let title = version_title(80, CLOSE_HINT, None).expect("a normal popup has room");
        let text: String = title
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(text, format!(" znicz {} ", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn an_available_update_shows_the_arrow() {
        let title = version_title(80, CLOSE_HINT, Some("9.9.9")).expect("room for the title");
        let text: String = title
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(
            text,
            format!(" znicz {} → 9.9.9 ", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn a_narrow_popup_drops_the_version_and_keeps_the_close_hint() {
        assert!(version_title(30, CLOSE_HINT, None).is_none());
        assert!(version_title(80, CLOSE_HINT, None).is_some());
    }
}
