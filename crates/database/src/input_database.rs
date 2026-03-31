use crate::File;
use crossbeam_channel::Sender;
use dashmap::{DashMap, Entry};
use notify_debouncer_mini::{
    new_debouncer,
    notify::{RecommendedWatcher, RecursiveMode},
    DebounceEventResult, Debouncer,
};
use salsa::{Database, Storage};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

#[salsa::db]
pub trait Db: Database {
    fn input(&self, path: PathBuf) -> Result<File, String>;
    fn clone_for_par(&self) -> Box<dyn Db>;
}

#[salsa::db]
impl Db for InputDatabase {
    fn clone_for_par(&self) -> Box<dyn Db> {
        Box::new(self.clone())
    }

    fn input(&self, path: PathBuf) -> Result<File, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        Ok(match self.files.entry(path.clone()) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let watcher = &mut *self.file_watcher.lock().unwrap();
                watcher
                    .watcher()
                    .watch(&path, RecursiveMode::NonRecursive)
                    .unwrap();
                let contents = std::fs::read_to_string(&path).unwrap();
                // Files loaded via import are always modules
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
    pub files: DashMap<PathBuf, File>,
    pub file_watcher: Arc<Mutex<Debouncer<RecommendedWatcher>>>,
}

impl InputDatabase {
    pub fn new(tx: Sender<DebounceEventResult>) -> Self {
        Self::new_with_debug_logging(tx, false)
    }

    pub fn new_with_debug_logging(tx: Sender<DebounceEventResult>, _debug: bool) -> Self {
        Self {
            storage: Storage::new(Some(Box::new({
                move |_event| {
                    #[cfg(debug_assertions)]
                    eprintln!("{_event:?}");
                }
            }))),
            files: DashMap::new(),
            file_watcher: Arc::new(Mutex::new(
                new_debouncer(Duration::from_secs(1), tx).unwrap(),
            )),
        }
    }
}
