use crate::buffer_pool::{BufferPool, DBReader, PAGE_SIZE, PageID, PageRef};
use crate::representations::page::{HeaderElem, InnerNode, Leaf, PageType, SlottedPage};
use crate::representations::tuple::{Tuple, TupleBuf};
use std::sync::Arc;

/// ScanIterator iterates over tuples of a page. 
/// 
/// After initialization with a starting page and the current slot idx being looked at,
/// the iterator will use sibling pointers to traverse rightward grabbing new pages. 
/// As it goes it checks if the end key has been reached. If not, it returns that iterator.
#[derive(Debug)]
struct ScanIterator<'a, R: DBReader> {
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
struct BTree<R: DBReader> {
    pool: Arc<BufferPool<R>>,
}

impl<R: DBReader> BTree<R> {
    /// Return an iterator to iterate over tuples in leaf nodes, using sibling pointers to
    /// move laterally
    fn iter_scan<'a>(
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

    fn insert_recurs(&self, page: PageRef<R>, tuple: &Tuple, mut parents: Vec<PageID>) {
        let read_lock = page.read().unwrap();
        let page_repr = SlottedPage::from_bytes(&read_lock);
        let room = page_repr.get_header(HeaderElem::FreeSpace);
        let page_type = PageType::new(page_repr.get_header(HeaderElem::PageType)).unwrap();

        // The required space is the size of the tuple plus the 2 byte slot ptr.
        // Check if we have enough room in the page.
        //
        // If we don't, then we need to:
        // (1) Split the page
        // (2) Insert a new key into the parent
        // (3) Possibly recurse that
        if tuple.size() as u32 + 2u32 > room {
            // Split and insert into parent. Check if we had a parent to get the leaf.
            // If we did, then use that as the parent id. Otherwise, we need to create a new page
            // and use that newly created page id
            let parent_id = match parents.pop() {
                Some(parent_id) => parent_id,
                None => {
                    // Create new page
                    let new_id = self.pool.new_page();
                    let new_page = self.pool.get_page_ref(new_id).unwrap();
                    InnerNode::from_bytes_mut(&mut new_page.write().unwrap()).init(new_id, 0, 0, 0);
                    new_id
                }
            };

            // Split page, updating sibling pointers
            let new_sibling_id = self.pool.new_page();
            let new_sibling_page = self.pool.get_page_ref(new_sibling_id).unwrap();

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
            }

            drop(read_lock);

            let mut write_lock = page.write().unwrap();
            let page_repr = SlottedPage::from_bytes_mut(&mut write_lock);

            let mut new_write_lock = new_sibling_page.write().unwrap();
            let new_page_repr = SlottedPage::from_bytes_mut(&mut new_write_lock);

            for moved_tuple in page_repr.split_half().iter() {
                new_page_repr.insert(&moved_tuple).unwrap();
            }

            // Update current page to point to new sibling on right

            // Push new key into parent, which will be tuple at index 0 of new page
            let new_key = new_page_repr.tuple(0).unwrap().key();
            let pointer = TupleBuf::new(new_key, &new_sibling_id.to_be_bytes());

            let parent = self.pool.get_page_ref(parent_id).unwrap();
            self.insert_recurs(parent, &pointer, parents);
        }

        // There is room - we can insert into the leaf
        Leaf::from_bytes_mut(&mut page.write().unwrap()).insert(tuple).unwrap();
    }

    fn insert_tuple(&self, page_root: PageID, tuple: &Tuple) {
        // Get leaf page if not leaf page
        let (leaf, parents) = self.get_leaf(
            self.pool.get_page_ref(page_root).unwrap(),
            tuple.key(),
            Vec::new(),
        );

        // Call insert page which may become recursive if parents need to be split
        self.insert_recurs(leaf, tuple, parents);
    }

    /// Get the leaf node of the tree containing the given key
    fn get_leaf<'a>(
        &'a self,
        page: PageRef<R>,
        key: &[u8],
        mut parents: Vec<PageID>,
    ) -> (PageRef<R>, Vec<PageID>) {
        let page_type = {
            let lock = page.read().unwrap();
            PageType::new(SlottedPage::from_bytes(&lock).get_header(HeaderElem::PageType)).unwrap()
        };
        match page_type {
            PageType::Leaf => (page, parents),
            PageType::Node => {
                let (next_id, this_id) = {
                    let lock = page.read().unwrap();
                    let repr = SlottedPage::from_bytes(&lock);
                    (
                        repr.get_header(HeaderElem::RightSiblingPtr),
                        repr.get_header(HeaderElem::PageID),
                    )
                };
                parents.push(this_id.into());
                self.get_leaf(
                    self.pool.get_page_ref(next_id.into()).unwrap(),
                    key,
                    parents,
                )
            }
        }
    }

    fn delete(&self) {
        todo!()
    }
}
