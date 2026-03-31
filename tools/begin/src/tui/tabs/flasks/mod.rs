mod landing;
mod results;
mod search_input;

pub use landing::*;
pub use results::*;
pub use search_input::*;

use ratatui::{buffer::Buffer, layout::Rect, widgets::StatefulWidgetRef};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum InputMode {
    #[default]
    Normal,
    Editing,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum FlasksState {
    #[default]
    Landing,
    // We keep the last query in case we send the same query twice we don't
    // actually need to make two web requests
    Results {
        query: String,
        selected: usize,
        tab: ResultTab,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Flasks {
    pub query_input: String,
    pub state: FlasksState,
}

pub const SEARCH_PLACEHOLDER: &str = " Type '/' to search ";

impl StatefulWidgetRef for Flasks {
    type State = InputMode;

    fn render_ref(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let query_input = self.query_input.clone();
        let flasks_state = self.state.clone();
        let input_mode = state.clone();

        use FlasksState::*;
        match flasks_state {
            Landing => {
                FlasksLanding.render_ref(area, buf, &mut (input_mode, query_input, flasks_state))
            }
            Results { tab, selected, .. } => {
                FlasksResults.render_ref(area, buf, &mut (input_mode, query_input, tab, selected));
            }
        }
    }
}
