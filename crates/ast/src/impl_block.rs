use crate::span::SpanId;
use std::hash::{Hash, Hasher};

use crate::MethodMap;
use internment::Intern;

/// A trait implementation block: `Args.Iterator (next: ...)`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplBlock {
    pub type_name: Intern<String>,
    pub type_name_span: SpanId,
    pub trait_name: Intern<String>,
    pub methods: MethodMap,
}

impl Hash for ImplBlock {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.trait_name.hash(state);
        let mut keys: Vec<_> = self.methods.keys().collect();
        keys.sort();
        for k in keys {
            k.hash(state);
            self.methods[k].hash(state);
        }
    }
}
