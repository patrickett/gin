use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Stylize,
    widgets::{Paragraph, StatefulWidget, Widget},
};

#[derive(Default, Clone, Copy)]
pub struct Title;

impl StatefulWidget for Title {
    type State = String;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // TODO: change begin version color depending on if we are out of date
        // YELLOW minor update
        // RED major update
        Paragraph::new(state.clone().bold()).render(area, buf);
        //  begin v0.1.0 (Latest: 1.0.0)
    }
}
