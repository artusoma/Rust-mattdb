use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, RwLock, RwLockWriteGuard},
    thread::LocalKey,
};

const PAGE_SIZE: usize = 8192;
const PAGES_IN_MEMORY: usize = 1000;

#[derive(Debug, Clone)]
pub struct Page {
    page_id: Option<u64>,
    content: [u8; PAGE_SIZE],
    pins: Option<u64>,
    is_dirty: bool,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            page_id: None,
            content: [0; PAGE_SIZE],
            pins: None,
            is_dirty: false,
        }
    }
}

impl Page {
    fn load_new(&mut self, page_id: u64, content: [u8; PAGE_SIZE]) {
        self.content = content;
        self.page_id = Some(page_id);
        self.is_dirty = false;
        self.pins = Some(0);
    }
}

/// Fat pointer to BufferPool with page id to be referenced attached
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
            .get(self.pool.idx_from_page_id(self.page_id))
            .unwrap()
    }
}

impl PageRef {}


/// Only way to rest of program to interact with pages.
/// Rest of matt-db cannot talk to disk -- it must talk to BufferPool.
///
/// Should be wrapped in an Arc
#[derive(Debug)]
pub struct BufferPool {
    //
    pages: Vec<RwLock<Page>>, // this stays fixed -- no need for a Mutex on `pages` (the Vec) itself. Could be a "boxed slice"
    id_to_idx: RwLock<HashMap<u64, usize>>,
    /// Least recently used tracker
    lru: RwLock<Vec<usize>>,
}

impl BufferPool {
    fn new() -> Self {
        Self {
            pages: std::iter::repeat_with(|| RwLock::new(Page::default()))
                .take(PAGES_IN_MEMORY)
                .collect(),
            id_to_idx: RwLock::new(HashMap::new()),
            lru: RwLock::new((1..PAGES_IN_MEMORY).into_iter().collect()),
        }
    }

    fn get_in_memory_page(&self, page_id: u64) -> &RwLock<Page> {
        let page_idx = self.idx_from_page_id(page_id);
        self.pages.get(page_idx).unwrap()
    }

    fn unpin(&self, page_id: u64) {
        if let Some(x) = self
            .get_in_memory_page(page_id)
            .write()
            .unwrap()
            .pins
            .as_mut()
        {
            *x -= 1;
        }
    }

    fn pin(&self, page_id: u64) {
        if let Some(x) = self
            .get_in_memory_page(page_id)
            .write()
            .unwrap()
            .pins
            .as_mut()
        {
            *x += 1;
        }
    }

    /// Only let people call this when managed by an Arc
    pub fn get_page_ref(self: &Arc<Self>, page_id: u64) -> Result<PageRef, String> {
        // Can't do this in match -> need to turn &usize into usize to avoid a deadlock
        let lookup_res = self.id_to_idx.read().unwrap().get(&page_id).copied();

        match lookup_res {
            Some(_) => {
                self.pin(page_id);
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
                    let mut page_lookup_write = self.id_to_idx.write().unwrap();
                    page_lookup_write.remove(&evict_id);
                    page_lookup_write.insert(page_id, evict_idx);
                }

                // Update page
                page_write.load_new(page_id, self.read_page(page_id));

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

    fn get_to_evict(&self) -> Result<usize, String> {
        // Get either (a) an empty spot or (b) the least recently used un-pinned page
        let lru_lock = self.lru.read().unwrap();
        let mut cidx = 0;
        let (page_idx, page_id) = loop {
            match lru_lock.get(cidx).unwrap() {
                Some(candidate_idx) => {
                    let page_lock = self.pages.get(*candidate_idx).unwrap().read().unwrap();
                    if page_lock.pins == Some(0) {
                        break (*candidate_idx, page_lock.page_id);
                    }
                }
                None => break (cidx, None),
            }

            cidx += 1;
            if cidx >= PAGES_IN_MEMORY {
                break (0, None);
            }
        };
        (page_idx, page_id)
    }

    fn write_page(&self, guard: &RwLockWriteGuard<Page>) {
        todo!()
    }

    fn idx_from_page_id(&self, page_id: u64) -> usize {
        *self.id_to_idx.read().unwrap().get(&page_id).unwrap()
    }
}
