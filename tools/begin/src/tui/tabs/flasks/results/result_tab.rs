use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
    // text::{Line, Text},
    widgets::{Paragraph, StatefulWidget, Tabs, Widget},
};
use strum::{Display, EnumIter, FromRepr, IntoEnumIterator};

#[derive(Debug, Default, Clone, Copy, Display, FromRepr, EnumIter, PartialEq, Eq)]
pub enum ResultTab {
    #[default]
    // #[strum(to_string = "Readme [r]")]
    #[strum(to_string = "Readme")]
    ReadMe,
    // #[strum(to_string = "Dependencies [d]")]
    #[strum(to_string = "Dependencies")]
    Dependencies,
    // #[strum(to_string = "Dependents [D]")]
    // #[strum(to_string = "Dependents")]
    // Dependents,
    // #[strum(to_string = "Versions [v]")]
    #[strum(to_string = "Versions")]
    Versions,
    // #[strum(to_string = "Security [s]")]
    #[strum(to_string = "Security")]
    Security,
}

impl ResultTab {
    pub fn title(self) -> String {
        format!(" {self} ")
    }

    // /// Get the previous tab, if there is no previous tab return the current tab.
    // pub fn previous(self) -> Self {
    //     let current_index: usize = self as usize;
    //     let previous_index = current_index.saturating_sub(1);
    //     Self::from_repr(previous_index).unwrap_or(self)
    // }

    // /// Get the next tab, if there is no next tab return the current tab.
    // pub fn next(self) -> Self {
    //     let current_index = self as usize;
    //     let next_index = current_index.saturating_add(1);
    //     Self::from_repr(next_index).unwrap_or(self)
    // }

    // /// Set the current tab to a specific index
    pub fn set_index(self, index: usize) -> Self {
        Self::from_repr(index).unwrap_or(self)
    }
}

#[derive(Default, Clone, Copy)]
/// The actual tabs of the tabs, not the content
pub struct ResultTabsDisplay;

/// ' | '
const TAB_PADDING: usize = 3;

impl StatefulWidget for ResultTabsDisplay {
    type State = ResultTab;

    // TODO: highlight the active tab when tab is custom rendered ie
    // when it can not fit
    // TODO: add keybinds bottom of window
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let titles = ResultTab::iter().map(ResultTab::title);
        let highlight_style = (Color::White, Color::DarkGray);
        let selected_tab_index = *state as usize;

        let can_fit = self.can_fit(area);

        if can_fit {
            Tabs::new(titles)
                // .block(Block::new().padding(Padding::top(1)))
                .highlight_style(highlight_style)
                .select(selected_tab_index)
                .render(area, buf);
        } else {
            let mut titles = titles.collect::<Vec<_>>();

            let mut lines = Vec::new();
            while !titles.is_empty() {
                let mut acc = 0;
                let idx = titles
                    .iter()
                    .take_while(|line| {
                        let w = line.len() + TAB_PADDING;
                        if acc + w <= (area.width as usize) {
                            acc += w;
                            true
                        } else {
                            false
                        }
                    })
                    .count();

                let displayable = titles.drain(..idx).collect::<Vec<_>>();
                lines.push(displayable.join(" | "));
            }

            Paragraph::new(lines.join("\n")).render(area, buf);
        }
    }
}

impl ResultTabsDisplay {
    pub fn width(&self) -> usize {
        ResultTab::iter()
            .map(ResultTab::title)
            .map(|i| i.len() + TAB_PADDING)
            .sum()
    }

    pub fn can_fit(&self, area: Rect) -> bool {
        self.width() <= (area.width as usize)
    }
}
