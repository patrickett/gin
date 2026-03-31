mod result_tab;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    widgets::{Block, BorderType, Paragraph, StatefulWidget, StatefulWidgetRef, Widget},
};
pub use result_tab::*;

use crate::tui::tabs::{InputMode, SearchInput};

#[derive(Debug, Clone, Default)]
pub struct FlasksResults;

impl StatefulWidgetRef for FlasksResults {
    type State = (InputMode, String, ResultTab, usize);

    fn render_ref(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use Constraint::{Length, Min, Percentage};

        let (input_mode, query_input, result_tabs, _selected_index) = state;

        let split_content = Layout::horizontal([Percentage(40), Percentage(60)]);
        let [left_area, right_area] = split_content.areas(area);

        let vertical = Layout::vertical([Length(3), Min(0)]);
        let [query_input_area, result_list_area] = vertical.areas(left_area);

        let q = query_input.clone();
        let im = input_mode.clone();
        SearchInput.render_ref(query_input_area, buf, &mut (im, q));

        let page_num = 1;

        Paragraph::new("result list area")
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .title_bottom(format!(" Page: {page_num} "))
                    .title_alignment(Alignment::Right),
            )
            .render(result_list_area, buf);

        let tabs_height = if ResultTabsDisplay.can_fit(right_area) {
            1
        } else {
            2
        };

        let vertical = Layout::vertical([Length(tabs_height), Min(0)]);
        let [result_tabs_area, result_tab_content] = vertical.areas(right_area);

        ResultTabsDisplay.render(result_tabs_area, buf, result_tabs);

        Paragraph::new("result tab content")
            .block(Block::bordered().border_type(BorderType::Rounded))
            .render(result_tab_content, buf);
    }
}
