use ratatui::text::Line;
use strum::{Display, EnumIter, FromRepr};

#[derive(Default, Clone, Copy, Display, FromRepr, EnumIter)]
pub enum SelectedTab {
    #[default]
    #[strum(to_string = "Info [1]")]
    Info,
    #[strum(to_string = "Build [2]")]
    Build,
    #[strum(to_string = "Docs [3]")]
    Docs,
    #[strum(to_string = "Flasks [4]")]
    Flasks,
}

impl SelectedTab {
    pub fn title(self) -> Line<'static> {
        format!("  {self}  ").into()
    }

    // TODO: change next_tab to loop instead of stopping
    // TODO: same with previous

    #[allow(unused)]
    /// Get the previous tab, if there is no previous tab return the current tab.
    pub fn previous(self) -> Self {
        let current_index: usize = self as usize;
        let previous_index = current_index.saturating_sub(1);
        Self::from_repr(previous_index).unwrap_or(self)
    }

    #[allow(unused)]
    /// Get the next tab, if there is no next tab return the current tab.
    pub fn next(self) -> Self {
        let current_index = self as usize;
        let next_index = current_index.saturating_add(1);
        Self::from_repr(next_index).unwrap_or(self)
    }

    /// Set the current tab to a specific index
    pub fn set_index(self, index: usize) -> Self {
        Self::from_repr(index).unwrap_or(self)
    }
}
