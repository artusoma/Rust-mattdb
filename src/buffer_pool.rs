use std::{
    collections::{HashMap, VecDeque},
    ffi::FromBytesUntilNulError,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, MutexGuard, RwLock, RwLockWriteGuard},
    thread::LocalKey,
};

const PAGE_SIZE: usize = 8192;
const PAGES_IN_MEMORY: usize = 1000;

// Types because I keep getting confused
type PageID = u64;
type Frame = usize;

#[derive(Debug, Clone)]
pub struct Page {
    page_id: Option<PageID>,
    content: [u8; PAGE_SIZE],
    pins: u64,
    is_dirty: bool,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            page_id: None,
            content: [0; PAGE_SIZE],
            pins: 0,
            is_dirty: false,
        }
    }
}

impl Page {
    fn load_new(&mut self, page_id: PageID, content: [u8; PAGE_SIZE]) {
        self.content = content;
        self.page_id = Some(page_id);
        self.is_dirty = false;
        self.pins = 0;
    }
}

/// Fat pointer to BufferPool with page id to be referenced attached
pub struct PageRef {
    pool: Arc<BufferPool>,
    page_id: PageID,
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

/// EvictManager handles evictions
#[derive(Debug)]
struct EvictManager {
    /// Queue of frames to evict
    evict_queue: Mutex<VecDeque<Frame>>,
    /// Stack of unused frames
    unused_frames: Mutex<Vec<Frame>>,
    /// Quick bool to check if any more unused frames
    unused_available: RwLock<bool>,
}

impl EvictManager {
    fn new(size: usize) -> Self {
        Self {
            evict_queue: Mutex::new(VecDeque::new()),
            unused_frames: Mutex::new((0..size).collect()),
            unused_available: RwLock::new(true),
        }
    }

    fn queue(&self) -> MutexGuard<VecDeque<Frame>> {
        self.evict_queue.lock().unwrap()
    }

    fn add_to_queue(&self, frame: Frame) {
        let mut queue = self.queue();
        if !queue.contains(&frame) {
            queue.push_back(frame)
        }
    }

    fn remove_from_queue(&self, frame: Frame) {
        let mut queue = self.queue();
        if let Some(idx) = queue.iter().position(|idx| *idx == frame) {
            queue.remove(idx);
        }
    }

    fn victim(&self) -> Option<Frame> {
        // Check if we have anything unused. 
        if *self.unused_available.read().unwrap() {
            if let Some(frame) = self.unused_frames.lock().unwrap().pop() {
                return Some(frame);
            } else {
                *self.unused_available.write().unwrap() = false
            }
        }
        self.queue().pop_front()
    }
}

/// Only way to rest of program to interact with pages.
/// Rest of matt-db cannot talk to disk -- it must talk to BufferPool.
///
/// Should be wrapped in an Arc
#[derive(Debug)]
pub struct BufferPool {
    /// this stays fixed -- no need for a Mutex on `pages` (the Vec) itself. Could be a "boxed slice"
    pages: Vec<RwLock<Page>>,
    id_to_idx: RwLock<HashMap<u64, usize>>,
    /// Least recently used tracker
    evict_manager: EvictManager,
    /// Unused frames
    unused_frames: Mutex<Vec<usize>>,
}

impl BufferPool {
    fn new() -> Self {
        Self {
            pages: std::iter::repeat_with(|| RwLock::new(Page::default()))
                .take(PAGES_IN_MEMORY)
                .collect(),
            id_to_idx: RwLock::new(HashMap::new()),
            evict_manager: EvictManager::new(),
            unused_frames: Mutex::new((0..PAGES_IN_MEMORY).collect()),
        }
    }

    fn get_in_memory_page(&self, page_id: u64) -> &RwLock<Page> {
        let page_idx = self.idx_from_page_id(page_id);
        self.pages.get(page_idx).unwrap()
    }

    fn unpin(&self, page_id: u64) {
        // Why not auto mut like in function signature?
        let mut page = self.get_in_memory_page(page_id).write().unwrap();
        page.pins -= 1;
        if page.pins == 0 {
            self.evict_manager.add_to_queue(page_id)
        }
    }

    fn pin(&self, page_id: u64) {
        let mut page = self.get_in_memory_page(page_id).write().unwrap();
        page.pins += 1;
        if page.pins == 1 {
            self.evict_manager.remove_from_queue(page_id);
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
