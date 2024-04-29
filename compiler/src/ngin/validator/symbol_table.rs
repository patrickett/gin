use crate::ngin::gin_type::GinType;
use std::collections::HashMap;
// fn build_symbol_table(ast: Vec<Node>) -> SymbolTable {
//     let stack: Vec<HashMap<String, Symbol>> = Vec::new();
//     SymbolTable { stack }
// }
pub enum ScopeKind {
    Local,
    Param,
    Global,
}

pub struct Symbol {
    scope_kind: ScopeKind,
    gin_type: GinType,
}

pub struct SymbolTable {
    pub stack: Vec<HashMap<String, Symbol>>,
}

impl SymbolTable {
    /// causes a new hash table to be pushed to the top of the stack (which is a new scope)
    fn enter(&mut self) {
        let map = HashMap::new();
        self.stack.push(map);
    }

    // bottom -> top
    // [x,x,x,x]
    //        ^ topmost

    /// causes the topmost hash table to be removed
    fn exit(&mut self) {
        self.stack.pop();
    }

    /// returns the number of hash tables in the current stack (helpful to know if in global scope)
    fn len(&self) -> usize {
        self.stack.len()
    }

    /// add entry to topmost hash table of the stack
    fn bind(&mut self, name: String, symbol: Symbol) {
        if let Some(table) = self.stack.last_mut() {
            table.insert(name, symbol);
        } else {
            panic!("No symbol table to bind 'variable' to")
        }
    }

    /// searches the stack of hash tables from top to bottom looking for first entry
    /// that matches name exactly
    fn lookup(&self, name: String) -> Option<&Symbol> {
        for table in self.stack.iter().rev() {
            if let Some(v) = table.get(&name) {
                return Some(v);
            }
        }
        None
    }

    /// same as `scope_lookup` but only searches within the topmost table
    fn lookup_topmost(&self, name: String) -> Option<&Symbol> {
        if let Some(table) = self.stack.last() {
            table.get(&name)
        } else {
            None
        }
    }
}
