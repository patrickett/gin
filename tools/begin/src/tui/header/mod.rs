mod title;
pub use title::*;

mod tabs_content;
pub use tabs_content::*;

mod selected_tab;
pub use selected_tab::*;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, StatefulWidget, StatefulWidgetRef, Widget},
};

#[derive(Default, Clone, Copy)]
pub struct Header;

impl StatefulWidgetRef for Header {
    type State = SelectedTab;

    fn render_ref(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use Constraint::{Length, Min};

        Block::new().borders(Borders::BOTTOM).render(area, buf);

        let mut app_version = format!("begin v{}", "0.1.0");
        let horizontal = Layout::horizontal([Min(0), Length(app_version.len() as u16)]);
        let [tabs_area, title_area] = horizontal.areas(area);

        TabsDisplay.render(tabs_area, buf, state);
        Title.render(title_area, buf, &mut app_version);
    }
}
