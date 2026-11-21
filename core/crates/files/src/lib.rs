//! File trees (the sidebar's Files view), headless: list one folder at a
//! time, watch the folders a tree has expanded, and read git status for
//! decorations.
//!
//! Nothing walks a whole tree up front. A tree lists its root, then each
//! folder the user expands; only expanded folders are watched for listings.
//! That keeps a 50k-file repository as cheap to open as a small one. Git
//! status also needs edits in closed folders: on macOS a work tree is
//! watched as a whole for that, which FSEvents does in one stream path.

mod git;
mod list;
mod sort;
mod watch;

pub use git::{GitState, GitStatus, status as git_status};
pub use list::{Entry, Listing, list, signature};
pub use sort::natural_cmp;
pub use watch::{Change, DirWatcher};

/// Most entries one listing returns; the rest are cut with `truncated`.
pub const MAX_ENTRIES: usize = 100_000;
