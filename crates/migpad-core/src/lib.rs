//! MigPad core: text storage, documents, encodings and line endings, saving, editing and undo,
//! the edit journal, search, session state, settings and localization.
//!
//! The core knows nothing about GPUI or windows, so it is tested and benchmarked on its own.

pub mod document;
pub mod encoding;
pub mod line_ending;
pub mod text;
