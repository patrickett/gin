// use crate::module::Module;

// ginc(path) -> Module (~/.gin_cache/mods/{modname})
// begin(path)
//  deps -> ginc -> Module
//  module -> binary

// Represents `~/.gin_cache/mods/` on disk.
// pub struct ModuleCache {
//     mods: Modules,
// }

// pub enum Modules {
//     /// Loaded means that we have read the
//     Loaded {
//         modules: Vec<Module>,
//         timestamp: i64, // TODO: replace with lastmodified on gin_cache/mods folder
//                         // err that might invalidate too much. we should check each module if its
//                         // timestamp/interface change and if so reload that specific module
//     },
//     Unloaded,
// }

// .gin_cache/
//   deps/
//      http@1.0.1/   # downloaded remote dependency
//          flask.json
//          http.gin
//   mods/
//      {fingerprint}-name/ # (hash of dir)
//          debug/
//              module.ast
//              module.lib
//          release/
