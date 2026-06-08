use std::{
    collections::{HashMap, VecDeque},
    ffi::{FromBytesUntilNulError, FromVecWithNulError},
    hash::Hash,
    io::Read,
    ops::{Deref, DerefMut, RangeBounds},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, RwLock, RwLockWriteGuard},
    thread::LocalKey,
};

const PAGE_SIZE: usize = 8192;
const PAGES_IN_MEMORY: usize = 1000;

// Types because I keep getting confused
pub type PageID = u64;
pub type Frame = usize;

#[derive(Debug, Clone)]
pub struct Page {
    page_id: Option<PageID>,
    content: [u8; PAGE_SIZE],
    pins: u64,
    is_dirty: bool,
}

impl Deref for Page {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

/// DerefMut is going to set the page dirty when something asks for it.
/// Make sures any mutable call is picked up as dirty.
impl DerefMut for Page {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.is_dirty = true;
        &mut self.content
    }
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
        self.pins = 1;
    }
}

/// Fat pointer to BufferPool with page id to be referenced attached
pub struct PageRef<R: DBReader> {
    pool: Arc<BufferPool<R>>,
    page_id: PageID,
}

/// When PageRef is dropped, decrement the page's pin count
impl<R: DBReader> Drop for PageRef<R> {
    fn drop(&mut self) {
        self.pool.unpin(self.page_id);
    }
}

impl<R: DBReader> Deref for PageRef<R> {
    type Target = RwLock<Page>;

    fn deref(&self) -> &Self::Target {
        self.pool.page(self.page_id)
    }
}

/// EvictManager handles evictions
#[derive(Debug)]
struct EvictManager {
    /// Queue of frames to evict
    evict_queue: Mutex<VecDeque<Frame>>,
    /// Stack of unused frames
    unused_frames: Mutex<Vec<Frame>>,
}

impl EvictManager {
    fn new(size: usize) -> Self {
        Self {
            evict_queue: Mutex::new(VecDeque::new()),
            unused_frames: Mutex::new((0..size).collect()),
        }
    }

    fn queue(&self) -> MutexGuard<'_, VecDeque<Frame>> {
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
        if let Some(frame) = self.unused_frames.lock().unwrap().pop() {
            return Some(frame);
        }
        self.queue().pop_front()
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BufferPoolError {
    #[error("No frames available in buffer.")]
    NoFreeFrames,
}

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

    fn offset(&self, page_id: PageID) -> u64 {
        page_id * PAGE_SIZE as u64
    }
}

/// Only way to rest of program to interact with pages.
/// Rest of matt-db cannot talk to disk -- it must talk to BufferPool.
///
/// Should be wrapped in an Arc
#[derive(Debug)]
pub struct BufferPool<R: DBReader> {
    /// The vector stays fixed -- no need for a Mutex on `pages` (the Vec) itself. Could be a "boxed slice".
    /// We need a RwLock on each Page, however -- because those threads can change.
    pages: Vec<RwLock<Page>>,
    /// Mapping from page id to frame
    page_table: RwLock<HashMap<PageID, Frame>>,
    /// Least recently used tracker
    evict_manager: EvictManager,
    /// Helper to read and write
    disk_io: R,
}

impl<R: DBReader> BufferPool<R> {
    fn new(disk: R, size: usize) -> Self {
        Self {
            pages: std::iter::repeat_with(|| RwLock::new(Page::default()))
                .take(size)
                .collect(),
            page_table: RwLock::new(HashMap::new()),
            evict_manager: EvictManager::new(size),
            disk_io: disk,
        }
    }

    fn page(&self, page_id: PageID) -> &RwLock<Page> {
        let page_idx = self.frame(page_id);
        self.pages.get(page_idx).unwrap()
    }

    fn unpin(&self, page_id: PageID) {
        // Why not auto mut like in function signature?
        let mut page = self.page(page_id).write().unwrap();
        page.pins -= 1;
        if page.pins == 0 {
            self.evict_manager.add_to_queue(self.frame(page_id))
        }
    }

    fn pin(&self, page_id: PageID) {
        let mut page = self.page(page_id).write().unwrap();
        page.pins += 1;
        if page.pins == 1 {
            self.evict_manager.remove_from_queue(self.frame(page_id));
        }
    }

    /// Only let people call this when managed by an Arc
    pub fn get_page_ref(self: &Arc<Self>, page_id: PageID) -> Result<PageRef<R>, BufferPoolError> {
        // Can't do this in match -> need to turn &usize into usize to avoid a deadlock
        let cache_res = self.page_table.read().unwrap().get(&page_id).copied();

        match cache_res {
            Some(_) => {
                self.pin(page_id);
                Ok(PageRef {
                    pool: Arc::clone(self),
                    page_id: page_id,
                })
            }
            None => {
                let evict_frame = self
                    .evict_manager
                    .victim()
                    .ok_or(BufferPoolError::NoFreeFrames)?;

                // Lock page
                let mut page_write = self.pages.get(evict_frame).unwrap().write().unwrap();

                if page_write.is_dirty {
                    self.write_page(&page_write);
                }

                // Wrap in its own scope to return the lock to not lock the DB during read
                {
                    let mut table_write = self.page_table.write().unwrap();
                    if let Some(old_page_id) = page_write.page_id {
                        table_write.remove(&old_page_id);
                    }
                    table_write.insert(page_id, evict_frame);
                }

                // Update load new page into frame
                page_write.load_new(page_id, self.read_page(page_id));

                Ok(PageRef {
                    pool: Arc::clone(self), //self.clone() can also work, but Arc::clone is explicit and safer
                    page_id: page_id,
                })
            }
        }
    }

    fn read_page(&self, page_id: PageID) -> [u8; PAGE_SIZE] {
        self.disk_io.read_page(page_id)
    }

    fn write_page(&self, guard: &RwLockWriteGuard<Page>) {
        self.disk_io
            .write_page(guard.page_id.unwrap(), &guard.content)
    }

    fn frame(&self, page_id: PageID) -> Frame {
        *self.page_table.read().unwrap().get(&page_id).unwrap()
    }

    fn new_page(self: &Arc<Self>) -> Result<PageRef<R>, BufferPoolError> {
        let new_id = self.disk_io.new_page();
        self.get_page_ref(new_id)
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    struct MockReader;

    impl MockReader {
        fn new() -> Self {
            Self {}
        }
    }

    impl DBReader for MockReader {
        fn write_page(&self, _page_id: PageID, _content: &[u8; PAGE_SIZE]) {
            // Definitely wrote a page here
        }

        fn read_page(&self, _page_id: PageID) -> [u8; PAGE_SIZE] {
            // Definitely read a real page here, really
            [0u8; PAGE_SIZE]
        }

        fn new_page(&self) -> PageID {
            todo!()
        }
    }

    #[test]
    fn test_evict_manager() {
        let evict = EvictManager::new(0);

        // Test add and pop
        evict.add_to_queue(1);
        assert_eq!(evict.victim(), Some(1));

        // Test now free frames are empty => None
        assert_eq!(evict.victim(), None);

        // Test add, add, pop
        evict.add_to_queue(0);
        evict.add_to_queue(5);
        assert_eq!(evict.victim(), Some(0));

        // Add 7, remove 5
        evict.add_to_queue(7);
        evict.remove_from_queue(5);
        assert_eq!(evict.victim(), Some(7));

        // Test popping off of queue again
        let evict = EvictManager::new(2);
        assert_eq!(evict.victim(), Some(1));
        assert_eq!(evict.victim(), Some(0));
        assert_eq!(evict.victim(), None);
    }

    #[test]
    fn test_buffer_pool() {
        // Create ARC of new pool
        let pool = Arc::new(BufferPool::new(MockReader::new(), 1));

        // Create copy (as a new thread would)
        let thread_pool = Arc::clone(&pool);

        // Create new ref
        let page_ref = thread_pool.get_page_ref(0).unwrap();
        assert_eq!(page_ref.read().unwrap().page_id, Some(0));

        // Use created frame
        let page_ref2 = thread_pool.get_page_ref(0).unwrap();
        assert_eq!(page_ref2.read().unwrap().page_id, Some(0));

        // Try to pull new pool in -> error! No frames left
        assert!(thread_pool.get_page_ref(1).is_err());

        // Even if we try to drop one ref, the other still is pinned:
        drop(page_ref);
        assert!(thread_pool.get_page_ref(1).is_err());

        // After unpinning both we are good to bring new page into memory
        drop(page_ref2);
        assert!(thread_pool.get_page_ref(1).is_ok());
    }
}
