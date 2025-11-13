use crate::frontend::prelude::*;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::path::PathBuf;

pub static COMPILER_CACHE: Lazy<DashMap<PathBuf, ParsedFolder>> = Lazy::new(DashMap::new);
