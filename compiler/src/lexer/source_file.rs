use std::{
    fs::{self, canonicalize},
    path::Path,
    time::SystemTime,
};

// having this file wrapper allows us to do some cool things in the future
// 1. dynamically mutate files based on user imput
// 2. reload files with their changes
pub struct SourceFile {
    // we keep the full path in case something else imports
    // so we can just reuse the same already parsed file
    full_path: String,
    content: String,
    last_modified: SystemTime,
}

impl SourceFile {
    pub fn full_path(&self) -> &String {
        &self.full_path
    }

    pub fn modified(&mut self) -> bool {
        let metadata = fs::metadata(self.full_path.to_string()).expect("user will have corrected");
        let recent_modified = metadata.modified().expect("has last modified date");

        if recent_modified > self.last_modified {
            self.last_modified = recent_modified;
            true
        } else {
            false
        }
    }

    pub fn new(path: String) -> Self {
        let path = Path::new(&path);
        if !path.exists() {
            // TODO: prompt user don't error
            eprintln!("No such file or directory: {}", path.display());
        }
        let path = canonicalize(path).expect("failed to get real path");
        if !path.exists() {
            // TODO: prompt user don't error
            eprintln!("No such file or directory: {}", path.display());
        }

        let full_path = path.to_str().expect("msg").to_string();

        // this is scary if the file is really large
        let content = fs::read_to_string(full_path.clone()).expect("msg");

        let metadata = fs::metadata(&full_path).expect("user will have corrected");
        let last_modified = metadata.modified().expect("has last modified date");

        Self {
            content,
            full_path,
            last_modified,
        }
    }

    // because in previous parts we will have confimred with the user
    // we can always return content
    pub fn content(&self) -> &String {
        &self.content
    }

    fn read_from_disk(&self) -> String {
        // TODO: check modified
        fs::read_to_string(self.full_path.clone()).expect("msg")
    }

    // if changes to file were made on disk, this allows us to re-lex-parse the file
    pub fn reload(&mut self) {
        if self.modified() {
            self.content = self.read_from_disk()
        } else {
        }
    }
}
