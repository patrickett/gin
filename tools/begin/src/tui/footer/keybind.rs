use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// display for the bottom

pub enum Action {
    /// a
    Add,
    /// d
    Delete,

    /// s,/
    Search,
    /// Esc
    Escape,
    /// Enter
    Submit,

    /// ^c, q
    Quit,
    DoNothing,
    // Arrows
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    SetTabIndex(usize),
    SetFlaskTabIndex(usize),

    Next,
}

impl From<KeyEvent> for Action {
    fn from(key: KeyEvent) -> Self {
        use Action::*;
        use KeyCode::*;
        let modifier = key.modifiers;
        let ctrl = modifier == KeyModifiers::CONTROL;

        match key.code {
            Char('c') => {
                if ctrl {
                    return Quit;
                }
                DoNothing
            }
            Char('q') => Quit,

            Char('/') => Search,

            Char('1') => SetTabIndex(0),
            Char('2') => SetTabIndex(1),
            Char('3') => SetTabIndex(2),
            Char('4') => SetTabIndex(3),

            Char('a') | Char('i') => Add,
            Char('d') | Char('u') => Action::Delete,

            Char('r') => SetFlaskTabIndex(0),
            Char('D') => SetFlaskTabIndex(1),
            // Char('D') => SetFlaskTabIndex(4),
            Char('v') => SetFlaskTabIndex(2),
            Char('s') => SetFlaskTabIndex(3),

            Left | Char('h') => MoveLeft,
            Right | Char('l') => MoveRight,
            Up | Char('k') => MoveUp,
            Down | Char('j') => MoveDown,
            Esc => Escape,
            Enter => Submit,
            Tab => Next,
            _ => DoNothing,
        }
    }
}

#[allow(unused)]
pub const NAV: &str = "Nav [←↓↑→]";

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Action::*;
        let v = match self {
            Next => "Next ⇥",
            Search => "Search [/]",
            Escape => "Escape [Esc]",
            Submit => "Submit [Enter]",
            Quit => "Quit [q|^c]",
            MoveUp => "Up [↑]",
            MoveDown => "Down [↓]",
            MoveLeft => "Left [←]",
            MoveRight => "Right [→]",
            SetTabIndex(_) => "Tab [1234]",
            SetFlaskTabIndex(_) => "Detail [rDvs]",
            _ => "",
        };

        write!(f, "{}", v)
    }
}
