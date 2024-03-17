use crate::expr::Expr;

#[derive(Debug, Clone)]
pub struct GinModule {
    // full_path: String,
    body: Vec<Expr>,
}

impl GinModule {
    // module might need path in the future,
    // but removing it for now as I am not actually using it.
    // SourceFile keeps state and will regenerate the Vec<Expr> body
    // when needed
    pub const fn new(body: Vec<Expr>) -> Self {
        Self { body }
    }

    // pub fn filename(&self) -> &str {
    //     self.path
    //         .file_stem()
    //         .expect("Getting file_stem from path")
    //         .to_str()
    //         .expect("converting file_stem to str")
    // }

    pub fn get_body(&self) -> &Vec<Expr> {
        &self.body
    }
}
