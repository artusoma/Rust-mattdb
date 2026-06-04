use std::{
    collections::{HashMap, VecDeque}, ffi::{FromBytesUntilNulError, FromVecWithNulError}, ops::{Deref, DerefMut}, path::PathBuf, sync::{Arc, Mutex, MutexGuard, RwLock, RwLockWriteGuard}, thread::LocalKey
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
        self.pool.pages.get(self.pool.frame(self.page_id)).unwrap()
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

    fn queue(&self) -> MutexGuard<VecDeque<Frame>> {
        self.evict_queue.lock().unwrap()
    }

    fn add_to_queue(&self, frame: Frame) {
        let mut queue = self.queue();
        if queue.contains(&frame) {
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

#[derive(Debug, thiserror::Error)]
pub enum BufferPoolError {
    #[error("No frames available to evict in buffer.")]
    NoFreeFrames,
}

struct DiskIO {
    file: PathBuf
}

impl DiskIO {
    fn write_page(&self, page_id: PageID, content: [u8; PAGE_SIZE]) {

    }

    fn read_page(&self, page_id: PageID) -> [u8; PAGE_SIZE] {
        todo!()
    }

    fn page_offset(&self, page_id: PageID) -> u8 {
        todo!()
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
    page_to_frame: RwLock<HashMap<PageID, Frame>>,
    /// Least recently used tracker
    evict_manager: EvictManager,
}

impl BufferPool {
    fn new() -> Self {
        Self {
            pages: std::iter::repeat_with(|| RwLock::new(Page::default()))
                .take(PAGES_IN_MEMORY)
                .collect(),
            page_to_frame: RwLock::new(HashMap::new()),
            evict_manager: EvictManager::new(PAGES_IN_MEMORY),
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
    pub fn get_page_ref(self: &Arc<Self>, page_id: PageID) -> Result<PageRef, BufferPoolError> {
        // Can't do this in match -> need to turn &usize into usize to avoid a deadlock
        let lookup_res = self.page_to_frame.read().unwrap().get(&page_id).copied();

        match lookup_res {
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
                    let mut page_lookup_write = self.page_to_frame.write().unwrap();
                    page_lookup_write.remove(&page_write.page_id.unwrap());
                    page_lookup_write.insert(page_id, evict_frame);
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
        todo!()
    }

    fn write_page(&self, guard: &RwLockWriteGuard<Page>) {
        todo!()
    }

    fn frame(&self, page_id: PageID) -> Frame {
        *self.page_to_frame.read().unwrap().get(&page_id).unwrap()
    }
}
