use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Stylize},
    text::Span,
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidgetRef, Widget},
};

use crate::tui::tabs::{InputMode, SEARCH_PLACEHOLDER};

#[derive(Debug, Clone, Default)]
pub struct SearchInput;

const SEARCH_TITLE: &str = "Search";

impl StatefulWidgetRef for SearchInput {
    type State = (InputMode, String);

    fn render_ref(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use InputMode::*;

        let (input_mode, query) = state;

        match input_mode {
            Normal => {
                let value = if query.is_empty() {
                    SEARCH_PLACEHOLDER
                } else {
                    query
                };

                Paragraph::new(value)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(Span::from(SEARCH_TITLE))
                            .border_type(BorderType::Rounded),
                    )
                    .render(area, buf)
            }
            Editing => Paragraph::new(query.as_str())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(Span::from(SEARCH_TITLE).bold())
                        .fg(Color::Green)
                        .border_type(BorderType::Rounded),
                )
                .render(area, buf),
        }
    }
}
