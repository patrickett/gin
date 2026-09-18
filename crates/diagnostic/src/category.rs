use ariadne::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Flaw,
    Warn,
    Help,
}

impl Category {
    pub fn color(&self) -> Color {
        match self {
            Self::Flaw => Color::Red,
            Self::Warn => Color::Yellow,
            Self::Help => Color::Blue,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Flaw => "flaw",
            Self::Warn => "warn",
            Self::Help => "help",
        }
    }
}
