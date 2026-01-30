use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
    widgets::{StatefulWidget, Tabs, Widget},
};
use strum::IntoEnumIterator;

use crate::tui::header::SelectedTab;

#[derive(Default, Clone, Copy)]
/// The actual tabs of the tabs, not the content
pub struct TabsDisplay;

impl StatefulWidget for TabsDisplay {
    type State = SelectedTab;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let titles = SelectedTab::iter().map(SelectedTab::title);
        let highlight_style = (Color::White, Color::DarkGray);
        let selected_tab_index = *state as usize;

        Tabs::new(titles)
            .highlight_style(highlight_style)
            .select(selected_tab_index)
            // .padding("", "|")
            // .divider("")
            .render(area, buf);
    }
}
