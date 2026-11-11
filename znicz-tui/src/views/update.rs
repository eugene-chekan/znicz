//! The Update modal: the new release's notes and what to do about them.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Clear;

use crate::app::App;
use crate::views;

pub fn render_modal(frame: &mut Frame, list: Rect, app: &mut App) {
    let popup = crate::layout::list_modal_rect(list);
    app.hits.overlay = Some(popup);
    app.hits.close = views::close_button_rect(popup);
    frame.render_widget(Clear, popup);
    let block = views::pane_block("Update", true, None).title(views::close_title());
    frame.render_widget(block, popup);
}
