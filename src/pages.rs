//! BTrees

use std::path::PathBuf;

pub struct Database {
    /// Size of page stored in bytes
    page_size: u64,
    /// Path to the database binary
    file: PathBuf,
}

pub struct BTree {}

pub struct BTreeNode {}

pub struct Pointer {
    block: u64,
    offset: u64
}

