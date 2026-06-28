use crate::buffer_pool::{BufferPool, DBReader, PageID, PageRef};
use crate::representations::page::{HeaderElem, InnerNode, Leaf, PageType, SlottedPage};
use crate::representations::tuple::{Tuple, TupleBuf};
use std::sync::Arc;

/// ScanIterator iterates over tuples of a page.
///
/// After initialization with a starting page and the current slot idx being looked at,
/// the iterator will use sibling pointers to traverse rightward grabbing new pages.
/// As it goes it checks if the end key has been reached. If not, it returns that iterator.
#[derive(Debug)]
pub struct ScanIterator<'a, R: DBReader> {
    pool: Arc<BufferPool<R>>,
    page: PageRef<R>,
    end_key: &'a [u8],
    idx: usize,
}

impl<'a, R: DBReader> ScanIterator<'a, R> {
    fn new(pool: Arc<BufferPool<R>>, page: PageRef<R>, end_key: &'a [u8], idx: usize) -> Self {
        Self {
            pool,
            page,
            end_key,
            idx,
        }
    }
}

impl<'a, R: DBReader> std::iter::Iterator for ScanIterator<'a, R> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        // Get read lock for page to check the header
        let lock = self.page.read().unwrap();
        let leaf = Leaf::from_bytes(&lock);
        let item_count = leaf.get_header(HeaderElem::ItemCount) as usize;
        let ptr = leaf.get_header(HeaderElem::RightSiblingPtr);
        drop(lock);

        // If we are at end of leaf, grab next page and reset idx
        if self.idx >= item_count {
            self.page = self.pool.get_page_ref(ptr).unwrap();
            self.idx = 0;
        }

        // Retake lock with new page
        let lock = self.page.read().unwrap();
        let leaf = Leaf::from_bytes(&lock);

        // Get next pair. Check key to see if we are at end.
        let t = leaf.tuple(self.idx).unwrap();
        self.idx += 1;
        if t.key() > self.end_key {
            None
        } else {
            Some(t.value().to_vec())
        }
    }
}

/// A B-tree index over a [`BufferPool`].
///
/// `BTree` manages page-level storage for ordered key-value data. Pages are
/// divided into two types: inner nodes that route lookups and leaf nodes that
/// store [`Tuple`] data. Leaf nodes are linked via right-sibling pointers,
/// enabling efficient range scans without ascending back to the root.
///
/// Insertions are handled recursively: when a page is full it is split and a
/// separator key is promoted to the parent, which may itself require splitting
/// up to the root.
///
/// # Type Parameters
///
/// * `R` - A [`DBReader`] that backs the [`BufferPool`] with persistent storage.
#[derive(Debug)]
pub struct BTree<R: DBReader> {
    pool: Arc<BufferPool<R>>,
}

impl<R: DBReader> BTree<R> {
    /// Return an iterator that iterates over tuples in leaf nodes,
    /// using sibling pointers to move laterally
    pub fn iter_scan<'a>(
        &'a self,
        page_root: PageID,
        start: &'a [u8],
        end: &'a [u8],
    ) -> ScanIterator<'a, R> {
        let page = self.pool.get_page_ref(page_root).unwrap();
        let (leaf, _) = self.get_leaf(page, start, Vec::new());

        // Get start index of search in page
        let start_idx = {
            let lock = leaf.read().unwrap();
            Leaf::from_bytes(&lock).find_key(start).unwrap()
        };
        ScanIterator::new(Arc::clone(&self.pool), leaf, end, start_idx)
    }

    /// Recursively inserts data into the page.
    ///
    /// The `page` argument may be a leaf node or a inner node
    fn insert_recurs(&self, page: PageRef<R>, tuple: &Tuple, mut parents: Vec<PageID>) {
        // Grab initially needed information
        let (page_space, page_type) = {
            let read_lock = page.read().unwrap();
            let page_repr = SlottedPage::from_bytes(&read_lock);
            let page_space = page_repr.get_header(HeaderElem::ContFreeSpace);
            let page_type = PageType::new(page_repr.get_header(HeaderElem::PageType)).unwrap();
            (page_space, page_type)
        };

        // The required space is the size of the tuple plus the 2 byte slot ptr.
        let required_space = tuple.size() as u32 + 2u32;

        // Check if we have enough room in the page.
        // If we don't, then we need to:
        // (1) Split the page
        // (2) Insert a new key into the parent
        // (3) Possibly recurse that
        let to_insert = if required_space > page_space {
            let (sibling_id, sibling_ptr) = self.split_page(&page, page_type);
            let parent_id = self.get_parent(&page, &mut parents);
            let parent = self.pool.get_page_ref(parent_id).unwrap();
            self.insert_recurs(parent, &sibling_ptr, parents);

            if tuple.key() < sibling_ptr.key() {
                page
            } else {
                self.pool.get_page_ref(sibling_id).unwrap()
            }
        } else {
            page
        };

        // There is room - we can insert into the leaf
        SlottedPage::from_bytes_mut(&mut to_insert.write().unwrap())
            .insert(tuple)
            .unwrap();
    }

    fn get_parent(&self, page: &PageRef<R>, parents: &mut Vec<u32>) -> u32 {
        let parent_id = match parents.pop() {
            Some(parent_id) => parent_id,
            None => {
                // Create new root. The left child ptr will be the current page id; sibling pointers are empty (0)
                let new_id = self.pool.new_page();
                let new_page = self.pool.get_page_ref(new_id).unwrap();
                InnerNode::from_bytes_mut(&mut new_page.write().unwrap()).init(
                    new_id,
                    0,
                    0,
                    page.id(),
                );
                new_id
            }
        };
        parent_id
    }

    fn split_page(&self, page: &PageRef<R>, page_type: PageType) -> (PageID, TupleBuf) {
        // Split page, updating sibling pointers
        let new_sibling_id = self.pool.new_page();
        let new_sibling_page = self.pool.get_page_ref(new_sibling_id).unwrap();

        // Based on the current page type, we need to split a page with that same type.
        // The left ptr will be the called page, and right ptr will be the called page's right ptr
        {
            let read_lock = page.read().unwrap();
            let page_repr = SlottedPage::from_bytes(&read_lock);
            match page_type {
                PageType::Leaf => Leaf::from_bytes_mut(&mut new_sibling_page.write().unwrap())
                    .init(
                        new_sibling_id,
                        page.id(),
                        page_repr.get_header(HeaderElem::RightSiblingPtr),
                    ),
                PageType::Node => InnerNode::from_bytes_mut(&mut new_sibling_page.write().unwrap())
                    .init(
                        new_sibling_id,
                        page.id(),
                        page_repr.get_header(HeaderElem::RightSiblingPtr),
                        0,
                    ),
            };
        }

        // Get page locks and representations for the left and right pages.
        // Move tuples from right into left
        let mut write_lock = page.write().unwrap();
        let page_repr = SlottedPage::from_bytes_mut(&mut write_lock);

        let mut new_write_lock = new_sibling_page.write().unwrap();
        let new_page_repr = SlottedPage::from_bytes_mut(&mut new_write_lock);

        for moved_tuple in page_repr.split_half().iter() {
            new_page_repr.insert(&moved_tuple).unwrap();
        }

        // Update current page to point to new sibling on right
        page_repr.set_header(HeaderElem::RightSiblingPtr, new_sibling_id);

        let sibling_key = new_page_repr.tuple(0).unwrap().key();
        (
            new_sibling_id,
            TupleBuf::new(sibling_key, &new_sibling_id.to_be_bytes()),
        )
    }

    pub fn insert_tuple(&self, page_root: PageID, tuple: &Tuple) {
        // Get leaf page if not leaf page
        let (leaf, parents) = self.get_leaf(
            self.pool.get_page_ref(page_root).unwrap(),
            tuple.key(),
            Vec::new(),
        );

        // Call insert page which may become recursive if parents need to be split
        self.insert_recurs(leaf, tuple, parents);
    }

    /// Traverses the tree from `page` downward, following child pointers in inner
    /// nodes until a leaf is found whose range covers `key`.
    ///
    /// Returns the leaf [`PageRef`] together with the ordered stack of [`PageID`]s
    /// for every inner node visited along the way (nearest ancestor last), which
    /// callers use when propagating splits upward.
    ///
    /// # Potential improvements
    ///
    /// * **Replace recursion with a loop** — each recursive call only tail-calls
    ///   itself, so the entire function body can be rewritten as a `loop { ... }`
    ///   with no stack growth.
    /// * **Single lock per node** — the current implementation acquires the read
    ///   lock twice per inner node: once to read the page type and again to read
    ///   the child pointer. Both values can be extracted inside a single lock scope
    ///   to halve the locking overhead.
    fn get_leaf(
        &self,
        page: PageRef<R>,
        key: &[u8],
        mut parents: Vec<PageID>,
    ) -> (PageRef<R>, Vec<PageID>) {
        let page_type = {
            let lock = page.read().unwrap();
            PageType::new(SlottedPage::from_bytes(&lock).get_header(HeaderElem::PageType)).unwrap()
        };
        match page_type {
            PageType::Leaf => {
                // If we found the leaf, just return the page and the current parent stack
                (page, parents)
            }
            PageType::Node => {
                // Get the child id
                let child_id = {
                    let lock = page.read().unwrap();
                    let repr = InnerNode::from_bytes(&lock);
                    repr.child(key)
                };

                // Push this onto the parents stack
                parents.push(page.id());

                // Recrusively call
                self.get_leaf(self.pool.get_page_ref(child_id).unwrap(), key, parents)
            }
        }
    }

    /// Delete a key from the B-Tree
    ///
    /// If this makes the page less than half full, then we need to check neighbors and maybe rearrange
    /// if they have any we could steal.
    ///
    /// As a first step, I think we can just always merge with neighbor.
    pub fn delete(&self) {
        todo!()
    }
}
