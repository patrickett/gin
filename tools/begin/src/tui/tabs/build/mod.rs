#![allow(unused)]

use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs, Widget, WidgetRef},
};

#[derive(Default, Clone, Copy)]
pub struct Build;

impl WidgetRef for Build {
    fn render_ref(&self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        Paragraph::new("Build page")
            // .block(self.block())
            .render(area, buf);
    }
}
