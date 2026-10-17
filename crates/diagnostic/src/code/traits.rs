pub trait Prefix {
    fn prefix(&self) -> &'static str {
        ""
    }
}

pub trait Code: Prefix {
    fn name(&self) -> &'static str;

    fn fmt_code(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = self.prefix();
        if !prefix.is_empty() {
            formatter.write_str(prefix)?;
            formatter.write_str("-")?;
        }
        formatter.write_str(self.name())
    }
}

pub(crate) struct CodeDisplay<'a, T: ?Sized>(pub &'a T);

impl<T: Code + ?Sized> std::fmt::Display for CodeDisplay<'_, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt_code(formatter)
    }
}
