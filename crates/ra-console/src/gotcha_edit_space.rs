//! The `edit-missed` gotcha's extra line for a replacement that ended in whitespace; the
//! provider adapter owns it, where the stream's edit results are scanned.

pub use ra_providers::claude_code::{TRAILING_SPACE_NOTE, edit_missed_message, ends_in_whitespace};
