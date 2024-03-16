use std::path::PathBuf;

use crate::expr::Expr;

#[derive(Debug, Clone)]
pub struct GinModule {
    path: PathBuf,
    body: Vec<Expr>,
}

impl GinModule {
    pub const fn new(path: PathBuf, body: Vec<Expr>) -> Self {
        Self { path, body }
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
