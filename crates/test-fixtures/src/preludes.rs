//! Source preludes that tests prepend to snippets so `finish` calls resolve.

pub const WORD_PRELUDE: &str = include_str!("../fixtures/word_prelude.gin");

pub const BYTE_PRELUDE: &str = include_str!("../fixtures/byte_prelude.gin");

pub const INT_PRELUDE: &str = include_str!("../fixtures/int_prelude.gin");
