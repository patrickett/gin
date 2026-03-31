mod footer;
mod header;
mod tabs;

use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    widgets::{StatefulWidgetRef, Widget, WidgetRef},
};

use crate::tui::{
    footer::{Action, Footer},
    header::{Header, SelectedTab},
    tabs::{Build, Docs, Flasks, FlasksState, Info, InputMode},
};

// TODO: maybe show program flow: https://ratatui.rs/showcase/third-party-widgets/#tui-nodes--

#[derive(Default)]
pub struct Tui {
    state: AppState,
    selected_tab: SelectedTab,
    input_mode: InputMode,

    info: Info,
    build: Build,
    docs: Docs,
    flasks: Flasks,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum AppState {
    #[default]
    Running,
    Quitting,
}

impl Tui {
    pub fn run(mut self, mut terminal: DefaultTerminal) -> std::io::Result<()> {
        while self.state == AppState::Running {
            terminal.draw(|frame| frame.render_widget(&self, frame.area()))?;
            self.handle_events()?;
        }
        Ok(())
    }

    // Char('l') | Right
    // Char('h') | Left
    fn handle_events(&mut self) -> std::io::Result<()> {
        use Action::*;
        use InputMode::*;
        use KeyCode::*;
        use SelectedTab::*;

        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let action = Action::from(key);

            match self.selected_tab {
                Info => {}
                Build => {}
                Docs => match self.input_mode {
                    Normal => match action {
                        Search => self.input_mode = Editing,
                        Submit => {
                            self.docs.state.toggle_selected();
                        }
                        Escape => {
                            // TODO: change to make sure this only happens when items is selected
                            self.docs.state.select(Vec::new());
                        }
                        MoveDown => {
                            self.docs.state.key_down();
                        }
                        MoveUp => {
                            self.docs.state.key_up();
                        }
                        MoveRight => {
                            self.docs.state.key_right();
                        }
                        MoveLeft => {
                            self.docs.state.key_left();
                        }
                        _ => (),
                    },
                    Editing => {
                        if let Escape = action {
                            self.docs.docs_query.clear();
                        }
                        if let Char(c) = key.code {
                            self.docs.docs_query.push(c);
                        } else if Backspace == key.code {
                            self.docs.docs_query.pop();
                        }
                    }
                },
                Flasks => match self.input_mode {
                    Normal => {
                        match self.flasks.state {
                            FlasksState::Landing => {}
                            FlasksState::Results { .. } => {
                                if let Escape = action {
                                    self.flasks.query_input.clear();
                                    self.flasks.state = FlasksState::Landing;
                                }
                            }
                        };
                        if let Search = action {
                            self.input_mode = Editing
                        }
                    }
                    Editing => {
                        match self.flasks.state {
                            FlasksState::Landing => {
                                if let Submit = action {
                                    self.flasks.state = tabs::FlasksState::Results {
                                        selected: 0,
                                        tab: tabs::ResultTab::ReadMe,
                                        query: self.flasks.query_input.clone(),
                                    };
                                    // TODO: send search query
                                }
                            }
                            FlasksState::Results { tab, .. } => {
                                if let SetFlaskTabIndex(index) = action {
                                    tab.set_index(index);
                                }
                            }
                        }

                        if let Char(c) = key.code {
                            self.flasks.query_input.push(c);
                        } else if Backspace == key.code {
                            self.flasks.query_input.pop();
                        }
                    }
                },
            }

            // Tab => self.next_tab(),
            // BackTab => self.previous_tab(),
            match self.input_mode {
                // Global keybinds when not in a text field
                Normal => match action {
                    SetTabIndex(index) => self.set_tab_index(index),
                    Quit => self.quit(),
                    _ => {}
                },
                // Global keybinds when in a text field
                Editing => match action {
                    Escape | Submit => self.input_mode = Normal,
                    _ => {}
                },
            };
        };
        Ok(())
    }

    pub fn set_tab_index(&mut self, index: usize) {
        self.selected_tab = self.selected_tab.set_index(index);
    }

    pub fn quit(&mut self) {
        self.state = AppState::Quitting;
    }
}

impl Widget for &Tui {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use Constraint::{Length, Min};
        use SelectedTab::*;
        let vertical = Layout::vertical([Length(2), Min(0), Length(1)]);
        let [header_area, content_area, footer_area] = vertical.areas(area);

        let mut selected_tab_copy = self.selected_tab;
        let mut input_mode = self.input_mode.clone();

        Header.render_ref(header_area, buf, &mut selected_tab_copy);
        match selected_tab_copy {
            Info => self.info.render_ref(content_area, buf),
            Build => self.build.render_ref(content_area, buf),
            Docs => self.docs.render_ref(content_area, buf, &mut input_mode),
            Flasks => self.flasks.render_ref(content_area, buf, &mut input_mode),
        }
        Footer.render(footer_area, buf);
    }
}
