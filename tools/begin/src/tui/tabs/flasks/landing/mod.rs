use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect, Spacing},
    style::Stylize,
    widgets::{Block, BorderType, Borders, Padding, Paragraph, StatefulWidgetRef, Widget},
};

use crate::tui::tabs::{FlasksState, InputMode, SearchInput};

#[derive(Debug, Clone, Default)]
pub struct FlasksLanding;

impl StatefulWidgetRef for FlasksLanding {
    type State = (InputMode, String, FlasksState);

    fn render_ref(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use Constraint::{Length, Min, Percentage};

        let (input_mode, query_input, _flasks_state) = state;

        let vertical = Layout::vertical([Length(3), Length(3), Min(0)]);
        let [title_area, query_input_area, content_area] = vertical.areas(area);

        Paragraph::new("The Gin Community's Flask Registry")
            .centered()
            .bold()
            .block(Block::new().padding(Padding::vertical(1)))
            .render(title_area, buf);

        let im = input_mode.clone();
        let q = query_input.clone();

        let search_layout = Layout::horizontal([Min(0), Percentage(50), Min(0)]);
        let [_, input_area, _] = search_layout.areas(query_input_area);

        SearchInput.render_ref(input_area, buf, &mut (im, q));

        let sections = Layout::horizontal([Percentage(33), Percentage(34), Percentage(33)])
            .spacing(Spacing::Space(2))
            .margin(2);
        let [new_area, updated_area, downloads_area] = sections.areas(content_area);

        Paragraph::new("")
            .block(
                Block::default()
                    .title(" New Flasks ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .render(new_area, buf);

        Paragraph::new("")
            .block(
                Block::default()
                    .title(" Most Downloads ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .render(downloads_area, buf);

        Paragraph::new("")
            .block(
                Block::default()
                    .title(" Recently Updated ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .render(updated_area, buf)
    }
}
