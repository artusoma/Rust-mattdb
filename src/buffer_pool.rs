use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, RwLock, RwLockWriteGuard},
    thread::LocalKey,
};

const PAGE_SIZE: usize = 8192;

#[derive(Debug)]
pub struct Page {
    page_id: u64,
    content: [u8; PAGE_SIZE],
    pins: u64,
    is_dirty: bool,
}

pub struct PageRef {
    pool: Arc<BufferPool>,
    page_id: u64,
}

/// When PageRef is dropped, decrement the page's pin count
impl Drop for PageRef {
    fn drop(&mut self) {
        self.pool.unpin(self.page_id);
    }
}

impl Deref for PageRef {
    type Target = RwLock<Page>;

    fn deref(&self) -> &Self::Target {
        self.pool
            .pages
            .get(self.pool.lookup_page_idx(self.page_id))
            .unwrap()
    }
}

impl PageRef {}

/// Only way to rest of program to interact with pages.
/// Rest of matt-db cannot talk to disk -- it must talk to BufferPool.
#[derive(Debug)]
pub struct BufferPool {
    pages: Vec<RwLock<Page>>, // this stays fixed -- no need for a Mutex on `pages` (the Vec) itself. Could be a "boxed slice"
    page_lookup: RwLock<HashMap<u64, usize>>,
    used_timer: RwLock<Vec<usize>>,
}

impl BufferPool {
    fn new() -> Self {
        todo!()
    }

    fn unpin(&self, page_id: u64) {
        let page_idx = self.lookup_page_idx(page_id);
        self.pages.get(page_idx).unwrap().write().unwrap().pins -= 1;
    }

    /// Only let people call this when managed by an Arc
    pub fn get_page_ref(self: &Arc<Self>, page_id: u64) -> Result<PageRef, String> {
        // Can't do this in match -> need to turn &usize into usize to avoid a deadlock
        let lookup_res = self.page_lookup.read().unwrap().get(&page_id).copied();

        match lookup_res {
            Some(idx) => {
                self.pages.get(idx).unwrap().write().unwrap().pins += 1;
                Ok(PageRef {
                    pool: Arc::clone(self),
                    page_id: page_id,
                })
            }
            None => {
                let (evict_idx, evict_id) = self.get_to_evict();

                // Lock page
                let mut page_write = self.pages.get(evict_idx).unwrap().write().unwrap();

                if page_write.is_dirty {
                    self.write_page(&page_write);
                }

                // Wrap in its own scope to return the lock to not lock the DB during read
                {
                    let mut page_lookup_write = self.page_lookup.write().unwrap();
                    page_lookup_write.remove(&evict_id);
                    page_lookup_write.insert(page_id, evict_idx);
                }

                // Update page
                page_write.content = self.read_page(page_id);
                page_write.pins = 1;
                page_write.page_id = page_id;
                page_write.is_dirty = false;

                Ok(PageRef {
                    pool: Arc::clone(self), //self.clone() can also work, but Arc::clone is explicit and safer
                    page_id: page_id,
                })
            }
        }
    }

    fn read_page(&self, page_id: u64) -> [u8; PAGE_SIZE] {
        todo!()
    }

    fn get_to_evict(&self) -> (usize, u64) {
        todo!()
    }

    fn write_page(&self, guard: &RwLockWriteGuard<Page>) {
        todo!()
    }

    fn lookup_page_idx(&self, page_id: u64) -> usize {
        *self.page_lookup.read().unwrap().get(&page_id).unwrap()
    }
}
