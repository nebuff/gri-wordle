//! The public-domain five-letter guess dictionary as sorted, fixed-width uppercase records.

pub const WORDS: &[u8] = include_bytes!("words.bin");
pub const WORD_BYTES: usize = 5;
pub const WORD_COUNT: usize = WORDS.len() / WORD_BYTES;
