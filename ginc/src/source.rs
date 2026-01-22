use std::{
    fs,
    path::{Path, PathBuf},
};

pub trait Source {
    fn content(&self) -> Vec<(String, PathBuf)>;
}

impl Source for &[u8] {
    fn content(&self) -> Vec<(String, PathBuf)> {
        let cu = (
            String::from_utf8(self.to_vec()).expect("Invalid UTF-8"),
            PathBuf::from(":memory:"),
        );

        vec![cu]
    }
}

impl Source for str {
    fn content(&self) -> Vec<(String, PathBuf)> {
        let cu = (self.to_string(), PathBuf::from(":memory:"));
        vec![cu]
    }
}

impl Source for &str {
    fn content(&self) -> Vec<(String, PathBuf)> {
        let cu = (self.to_string(), PathBuf::from(":memory:"));
        vec![cu]
    }
}

impl Source for String {
    fn content(&self) -> Vec<(String, PathBuf)> {
        let cu = (self.to_string(), PathBuf::from(":memory:"));
        vec![cu]
    }
}

impl Source for Path {
    fn content(&self) -> Vec<(String, PathBuf)> {
        if self.is_file() {
            let cu = (
                fs::read_to_string(self).expect("file exists"),
                PathBuf::from(":memory:"),
            );
            vec![cu]
        } else {
            let mut contents = Vec::new();
            let mut readdir = fs::read_dir(self).expect("directory needs to exist");

            while let Some(Ok(dirent)) = readdir.next() {
                let content = fs::read_to_string(dirent.path()).expect("file has content");
                contents.push((content, dirent.path()))
            }
            contents
        }
    }
}

impl Source for PathBuf {
    fn content(&self) -> Vec<(String, PathBuf)> {
        if self.is_file() {
            let cu = (
                fs::read_to_string(self).expect("file exists"),
                PathBuf::from(":memory:"),
            );
            vec![cu]
        } else {
            let mut contents = Vec::new();
            let mut readdir = fs::read_dir(self).expect("directory needs to exist");

            while let Some(Ok(dirent)) = readdir.next() {
                let content = fs::read_to_string(dirent.path()).expect("file has content");
                contents.push((content, dirent.path()))
            }
            contents
        }
    }
}
