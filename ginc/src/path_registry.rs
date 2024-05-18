use bimap::BiMap;
use std::path::Path;

#[derive(PartialEq)]
pub struct PathRegistry {
    paths: BiMap<String, usize>,
}

impl PathRegistry {
    pub fn new() -> Self {
        PathRegistry {
            paths: BiMap::new(),
        }
    }

    pub fn insert(&mut self, path: &str) -> usize {
        // Relative path
        let relative_path = Path::new(path);

        // Convert relative path to full path
        if let Ok(full_path) = std::fs::canonicalize(relative_path) {
            let display_path = full_path.display().to_string();

            if let Some(&index) = self.paths.get_by_left(&display_path) {
                return index;
            }

            let index = self.paths.len();
            // self.paths
            self.paths.insert(display_path, index);
            index
        } else {
            panic!("Failed to get full path");
        }
    }

    pub fn lookup(&self, index: usize) -> Option<&String> {
        self.paths.get_by_right(&index)
    }
}
