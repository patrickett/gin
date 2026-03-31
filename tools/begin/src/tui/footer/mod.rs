mod keybind;
pub use keybind::*;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Block, Padding, Paragraph, Widget},
};

#[derive(Default, Clone, Copy)]
pub struct Footer;

impl Widget for Footer {
    fn render(self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        // TODO: handle keybinds so we can never be wrong and dont need update more
        // than once place

        Paragraph::new("Quit [^c]")
            .block(Block::default().padding(Padding::horizontal(1)))
            .render(area, buf);
    }
}
