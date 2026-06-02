//! BTrees

use std::path::PathBuf;

pub struct Database {
    /// Size of page stored in bytes
    page_size: u64,
    /// Path to the database binary
    file: PathBuf,
}

impl Database {
    /// Returns error if file is a directory instead of a file
    fn new(page_size: u64, file: PathBuf) -> Result<Self, std::io::Error> {
        if !file.is_file() {
            // do something!
            todo!()
        }
        Ok(Self { page_size, file })
    }
}

pub struct BTree {}

pub struct BTreeNode {}

pub struct Pointer {
    block: u64,
    offset: u64,
}
