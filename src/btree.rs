use crate::buffer_pool::{BufferPool, DBReader, PageID, PageRef};
use crate::representations::page::{
    HeaderElem, InnerNode, Leaf, NULL_PTR, PageReadWriteError, PageType, SlottedPage,
};
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
        let (ptr, item_count) = {
            let lock = self.page.read().unwrap();
            let leaf = Leaf::from_bytes(&lock);
            let item_count = leaf.get_header(&HeaderElem::ItemCount) as usize;
            let ptr = leaf.get_header(&HeaderElem::RightSiblingPtr);
            (ptr, item_count)
        };

        if ptr == NULL_PTR {
            return None;
        }

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
        if t.key().bytes() > self.end_key {
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
    ///
    /// # To-do
    /// \[ ] Check for a tuple overflow when we do a final insert after inserting into
    ///     parent. Right now we unwrap, which may cause issues later.
    fn insert_recurs(&self, page: PageRef<R>, tuple: &Tuple, mut parents: Vec<PageID>) {
        // Try an insert and store the result
        let insert_result = {
            let mut write_lock = page.write().unwrap();
            let page_repr = SlottedPage::from_bytes_mut(&mut write_lock);
            page_repr.insert(tuple)
        };

        // Check the result. If it is Ok, then we inserted!
        // If not, we need to get the parent and insert there.
        match insert_result {
            Ok(_) => {}
            Err(PageReadWriteError::OutOfSpace) => {
                // Do an insert and split, getting what we need to insert into parent
                let upstream_key = self.split_and_insert(&page, &tuple);

                // This parent will either be an existing node in the tree,
                // or a new parent node returned will have a left pointer to the original
                // page that we split.
                let parent_id = self.get_parent(&page, &mut parents);
                let parent_ref = self.pool.get_page_ref(parent_id).unwrap();

                // Now insert the sibling pointer into the parent that we got
                self.insert_recurs(parent_ref, &upstream_key, parents);
            }
            // Should not get anything else
            Err(e) => unreachable!("unexpected insert error: {e:?}"),
        }
    }

    fn get_parent(&self, page: &PageRef<R>, parents: &mut Vec<PageID>) -> PageID {
        match parents.pop() {
            Some(parent_id) => parent_id,
            None => {
                // Create new root. The left child ptr will be the current page id; sibling pointers are empty (NULL_PTR)
                let new_id = self.pool.new_page();
                let new_page = self.pool.get_page_ref(new_id).unwrap();
                InnerNode::from_bytes_mut(&mut new_page.write().unwrap()).init(
                    new_id,
                    NULL_PTR,
                    NULL_PTR,
                    page.id(),
                );
                new_id
            }
        }
    }

    /// Splits the page and inserts the tuple, returning the tuple that needs to be inserted
    /// into the parent.
    fn split_and_insert(&self, page: &PageRef<R>, tuple: &Tuple) -> TupleBuf {
        // Split page, updating sibling pointers
        let new_sibling_id = self.pool.new_page();
        let new_sibling_page = self.pool.get_page_ref(new_sibling_id).unwrap();

        // Get locks
        let mut write_lock = page.write().unwrap();
        let mut new_write_lock = new_sibling_page.write().unwrap();

        // Check if we looking at a leaf or a node.
        // If a leaf, we need to just split and keep everything.
        // If a node, we need to take the middle key, then split
        match SlottedPage::from_bytes_mut(&mut write_lock)
            .get_header(&HeaderElem::PageType)
            .try_into()
            .unwrap()
        {
            PageType::Leaf => {
                let left_page_repr = Leaf::from_bytes_mut(&mut write_lock);
                let right_page_repr = Leaf::from_bytes_mut(&mut new_write_lock);

                let (middle_tuple, right_tuples) = left_page_repr.split_half(tuple);

                for moved_tuple in right_tuples.iter() {
                    right_page_repr.insert(&moved_tuple).unwrap();
                }

                // Init sibling page
                // The new left child ptr needs to be the page that the promoted
                // middle key used to point to
                right_page_repr.init(
                    new_sibling_id,
                    page.id(),
                    left_page_repr.get_header(&HeaderElem::RightSiblingPtr),
                );

                match tuple.key().bytes().cmp(middle_tuple.key().bytes()) {
                    std::cmp::Ordering::Less => left_page_repr.insert(tuple).unwrap(),
                    _ => right_page_repr.insert(tuple).unwrap(),
                }

                left_page_repr.set_header(
                    &HeaderElem::RightSiblingPtr,
                    right_page_repr.get_header(&HeaderElem::PageID),
                );

                middle_tuple
            }

            PageType::Node => {
                let left_page_repr = InnerNode::from_bytes_mut(&mut write_lock);
                let right_page_repr = InnerNode::from_bytes_mut(&mut new_write_lock);

                let (middle_tuple, right_tuples) = left_page_repr.split_half(tuple);

                for moved_tuple in right_tuples.iter() {
                    right_page_repr.insert(&moved_tuple).unwrap();
                }

                // Init sibling page
                // The new left child ptr needs to be the page that the promoted
                // middle key used to point to
                right_page_repr.init(
                    new_sibling_id,
                    page.id(),
                    left_page_repr.get_header(&HeaderElem::RightSiblingPtr),
                    u32::from_be_bytes(middle_tuple.value().try_into().unwrap()),
                );

                match tuple.key().bytes().cmp(middle_tuple.key().bytes()) {
                    // Insert into left
                    std::cmp::Ordering::Less => left_page_repr.insert(tuple).unwrap(),
                    std::cmp::Ordering::Equal => {}
                    std::cmp::Ordering::Greater => right_page_repr.insert(tuple).unwrap(),
                }

                left_page_repr.set_header(
                    &HeaderElem::RightSiblingPtr,
                    right_page_repr.get_header(&HeaderElem::PageID),
                );

                middle_tuple
            }
        }
    }

    pub fn insert_tuple(&self, page_root: PageID, tuple: &Tuple) {
        // Get leaf page if not leaf page
        let (leaf, parents) = self.get_leaf(
            self.pool.get_page_ref(page_root).unwrap(),
            tuple.key().bytes(),
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
            PageType::new(SlottedPage::from_bytes(&lock).get_header(&HeaderElem::PageType)).unwrap()
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
