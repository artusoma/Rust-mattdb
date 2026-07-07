use crate::buffer_pool::{PAGE_SIZE, PageID};
use std::{collections::HashMap, path::PathBuf};
use std::sync::{Mutex};

pub trait DBReader {
    fn write_page(&self, page_id: PageID, content: &[u8; PAGE_SIZE]);
    fn read_page(&self, page_id: PageID) -> [u8; PAGE_SIZE];
    fn new_page(&self) -> PageID;
}

/// Handles reading and writing from disk
#[derive(Debug)]
pub struct DiskIO {
    file: PathBuf,
    pages: usize,
}

impl DBReader for DiskIO {
    fn read_page(&self, page_id: PageID) -> [u8; PAGE_SIZE] {
        todo!()
    }

    fn write_page(&self, page_id: PageID, content: &[u8; PAGE_SIZE]) {
        todo!()
    }

    fn new_page(&self) -> PageID {
        todo!()
    }
}

impl DiskIO {
    fn new() -> Self {
        DiskIO {
            file: PathBuf::new(),
            pages: 0,
        }
    }

    fn offset(&self, page_id: PageID) -> usize {
        page_id as usize * PAGE_SIZE
    }
}

/// Handles in-memory management
#[derive(Debug)]
pub struct MemoryIO {
    pages: Mutex<HashMap<PageID, [u8; PAGE_SIZE]>,>
}

impl Default for MemoryIO {
    fn default() -> Self {
        Self {
            pages: Mutex::new(HashMap::new()),
        }
    }
}

impl DBReader for MemoryIO {
    /// Arrays implement copy when their child element does,
    /// so this copies.
    fn read_page(&self, page_id: PageID) -> [u8; PAGE_SIZE] {
        *self.pages.lock().unwrap().get(&page_id).unwrap()
    }

    fn new_page(&self) -> PageID {
        let mut page_lock = self.pages.lock().unwrap();
        let new_id = (page_lock.len() + 1) as u32;
        page_lock.insert(new_id, [0u8; PAGE_SIZE]);
        new_id
    }

    fn write_page(&self, page_id: PageID, content: &[u8; PAGE_SIZE]) {
        self.pages.lock().unwrap().insert(page_id, *content);
    }
}
