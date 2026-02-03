use crate::database::File;
use crossbeam_channel::Sender;
use dashmap::{DashMap, Entry};
use notify_debouncer_mini::{
    DebounceEventResult, Debouncer, new_debouncer,
    notify::{RecommendedWatcher, RecursiveMode},
};
use salsa::{Database, Storage};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[salsa::db]
pub trait Db: Database {
    // Error = Report
    fn input(&self, path: PathBuf) -> Result<File, String>;
}

#[salsa::db]
impl Db for InputDatabase {
    fn input(&self, path: PathBuf) -> Result<File, String> {
        let path = path.canonicalize().unwrap();
        // .wrap_err_with(|| format!("Failed to read {}", path.display()))?;
        Ok(match self.files.entry(path.clone()) {
            // If the file already exists in our cache then just return it.
            Entry::Occupied(entry) => *entry.get(),
            // If we haven't read this file yet set up the watch, read the
            // contents, store it in the cache, and return it.
            Entry::Vacant(entry) => {
                // Set up the watch before reading the contents to try to avoid
                // race conditions.
                let watcher = &mut *self.file_watcher.lock().unwrap();
                watcher
                    .watcher()
                    .watch(&path, RecursiveMode::NonRecursive)
                    .unwrap();
                let contents = std::fs::read_to_string(&path).unwrap();
                // .wrap_err_with(|| format!("Failed to read {}", path.display()))?;
                *entry.insert(File::new(self, path, contents))
            }
        })
    }
}

#[salsa::db]
impl Database for InputDatabase {}

#[salsa::db]
#[derive(Clone)]
pub struct InputDatabase {
    pub storage: Storage<Self>,
    // pub logs: Arc<Mutex<Vec<String>>>,
    pub files: DashMap<PathBuf, File>,
    pub file_watcher: Arc<Mutex<Debouncer<RecommendedWatcher>>>,
}

impl InputDatabase {
    pub fn new(tx: Sender<DebounceEventResult>) -> Self {
        Self::new_with_debug_logging(tx, false)
    }

    pub fn new_with_debug_logging(tx: Sender<DebounceEventResult>, _debug: bool) -> Self {
        // let logs: Arc<Mutex<Vec<String>>> = Default::default();
        Self {
            storage: Storage::new(Some(Box::new({
                // let logs = logs.clone();
                move |event| {
                    eprintln!("{event:?}");
                }
            }))),
            // logs,
            files: DashMap::new(),
            file_watcher: Arc::new(Mutex::new(
                new_debouncer(Duration::from_secs(1), tx).unwrap(),
            )),
        }
    }
}
