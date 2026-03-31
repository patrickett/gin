#![allow(unused)]

use std::path::PathBuf;

use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph, Tabs, Widget, WidgetRef},
};

#[derive(Default, Clone)]
pub enum InfoState {
    #[default]
    ManifestNotFound,
    Flask {
        path: PathBuf,
    },
}

#[derive(Default, Clone)]
pub struct Info {
    state: InfoState,
}

// module(project) name (snake case)
// Source Lines Of Code (excluding tests comments and example code)
// last updated
// authors
// dependencies
// repository (optional)
// size on disk
// version and number of versions

impl WidgetRef for Info {
    fn render_ref(&self, area: Rect, buf: &mut Buffer)
    where
        Self: Sized,
    {
        let info_lines = vec![
            Line::from("Module Name: gin"),
            Line::from("Source Lines Of Code: 0"),
            Line::from("Last Updated: 2026-01-27"),
            Line::from("Authors: Unknown"),
            Line::from("Dependencies: None"),
            Line::from("Repository: https://github.com/example/gin"),
            Line::from("Size on Disk: 0 KB"),
            Line::from("Version: 0.1.0 (1 versions)"),
        ];

        Paragraph::new(info_lines)
            .block(
                Block::default()
                    .title(" Project Info ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .render(area, buf);
    }
}
