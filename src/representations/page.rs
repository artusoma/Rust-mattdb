use std::ops::{Deref, DerefMut};

use super::tuple::*;
use crate::buffer_pool::{PAGE_SIZE, PageID};

#[derive(Debug, thiserror::Error)]
pub enum PageReadWriteError {
    #[error("Page is out of space for insert")]
    OutOfSpace,
    #[error("Cannot find specified key (bytes: {0})")]
    KeyNotFound(String),
    #[error("Page type error: cannot interpret page type `{0}`")]
    PageTypeError(u32),
}

pub enum HeaderElem {
    PageID,
    PageType,
    ContFreeSpace,
    LastCommit,
    ItemCount,
    LeftSiblingPtr,
    RightSiblingPtr,
    FreeSpacePtr,
    LeftChildPtr,
    TotalFreeSpace,
}

impl HeaderElem {
    fn offset(&self) -> usize {
        match self {
            Self::PageID => 0,
            Self::PageType => 4,
            Self::ContFreeSpace => 8,
            Self::LastCommit => 12,
            Self::ItemCount => 16,
            Self::LeftSiblingPtr => 20,
            Self::RightSiblingPtr => 24,
            Self::FreeSpacePtr => 28,
            Self::LeftChildPtr => 32,
            Self::TotalFreeSpace => 36,
        }
    }
}

const HEADER_SIZE: usize = 4 * 10;

/// Page type in the BTree. Node pages map keys to page ids, leaf pages map keys to tuples
#[derive(Debug, PartialEq)]
pub enum PageType {
    /// Nodes always map to Key => PageID
    Node,
    /// Leaf always maps Key => Tuple
    Leaf,
}

impl PageType {
    pub fn id(&self) -> u32 {
        match self {
            Self::Node => 0,
            Self::Leaf => 1,
        }
    }

    pub fn new(type_id: u32) -> Result<Self, PageReadWriteError> {
        match type_id {
            0 => Ok(Self::Node),
            1 => Ok(Self::Leaf),
            _ => Err(PageReadWriteError::PageTypeError(type_id)),
        }
    }
}

impl TryFrom<u32> for PageType {
    type Error = PageReadWriteError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        PageType::new(value)
    }
}

/// Represents space available on an insert
enum SpaceStatus {
    Ok,
    NeedsCollapse,
    OutOfSpace,
}

/// Single shared representation for a slotted page and its ensuing operations
#[repr(transparent)]
pub struct SlottedPage([u8]);

impl SlottedPage {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const Self) }
    }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        unsafe { &mut *(bytes as *mut [u8] as *mut Self) }
    }

    pub fn get_header(&self, element: &HeaderElem) -> u32 {
        let offset = element.offset();
        u32::from_be_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }

    pub fn set_header(&mut self, element: &HeaderElem, value: u32) {
        let offset = element.offset();
        self.0[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    pub fn update_header(&mut self, element: &HeaderElem, diff: i64) {
        let current = self.get_header(element);
        self.set_header(element, ((current as i64) + diff) as u32);
    }

    pub fn percent_full(&self) -> u8 {
        100 - (self.get_header(&HeaderElem::ContFreeSpace) as usize * 100
            / (PAGE_SIZE - HEADER_SIZE)) as u8
    }

    pub fn init(
        &mut self,
        page_id: PageID,
        page_type: PageType,
        left_ptr: PageID,
        right_ptr: PageID,
        left_child_ptr: Option<PageID>,
    ) {
        self.set_header(&HeaderElem::PageID, page_id.try_into().unwrap());
        self.set_header(&HeaderElem::PageType, page_type.id().try_into().unwrap());
        self.set_header(
            &HeaderElem::ContFreeSpace,
            (PAGE_SIZE - HEADER_SIZE).try_into().unwrap(),
        ); // 2 bytes for first slot
        self.set_header(
            &HeaderElem::TotalFreeSpace,
            (PAGE_SIZE - HEADER_SIZE).try_into().unwrap(),
        ); // 2 bytes for first slot
        self.set_header(&HeaderElem::ItemCount, 0);
        self.set_header(&HeaderElem::LeftSiblingPtr, left_ptr.try_into().unwrap());
        self.set_header(&HeaderElem::RightSiblingPtr, right_ptr.try_into().unwrap());
        self.set_header(&HeaderElem::FreeSpacePtr, PAGE_SIZE.try_into().unwrap());

        if let Some(x) = left_child_ptr {
            self.set_header(&HeaderElem::LeftChildPtr, x.try_into().unwrap())
        };
    }

    pub fn tuple(&self, idx: usize) -> Option<&Tuple> {
        // Read first u16 / get size
        let ptr = if self.get_header(&HeaderElem::ItemCount) <= idx as u32 {
            None
        } else {
            let slot_ptr = HEADER_SIZE + idx * 2;
            Some(u16::from_be_bytes(self.0[slot_ptr..slot_ptr + 2].try_into().unwrap()) as usize)
        }?;
        let tuple_size = u16::from_be_bytes(self.0[ptr..ptr + 2].try_into().unwrap()) as usize;
        Some(Tuple::from_bytes(&self.0[ptr..ptr + tuple_size]))
    }

    pub fn find_key(&self, key: &[u8]) -> Option<usize> {
        let count = self.get_header(&HeaderElem::ItemCount);
        if count == 0 {
            None
        } else {
            self.find_key_inner(key, 0, self.get_header(&HeaderElem::ItemCount) as usize - 1)
        }
    }

    fn find_key_inner(&self, key: &[u8], low: usize, high: usize) -> Option<usize> {
        let idx = low + (high - low) / 2;

        if high == low {
            return if key == self.tuple(high).unwrap().key() {
                Some(high)
            } else {
                None
            };
        }

        let this_key = self.tuple(idx).unwrap().key();

        match this_key.cmp(key) {
            std::cmp::Ordering::Equal => Some(idx),
            std::cmp::Ordering::Less => self.find_key_inner(key, idx + 1, high),
            std::cmp::Ordering::Greater => {
                if idx == 0 {
                    return None;
                } else {
                    self.find_key_inner(key, low, idx - 1)
                }
            }
        }
    }

    pub fn find_partition(&self, key: &[u8]) -> usize {
        let count = self.get_header(&HeaderElem::ItemCount);
        if count == 0 {
            0
        } else {
            self.find_partition_inner(key, 0, self.get_header(&HeaderElem::ItemCount) as usize - 1)
        }
    }

    fn find_partition_inner(&self, key: &[u8], low: usize, high: usize) -> usize {
        let idx = low + (high - low) / 2;

        let this_key = self.tuple(idx).unwrap().key();
        if high == low {
            match this_key.cmp(key) {
                std::cmp::Ordering::Less => idx + 1,
                std::cmp::Ordering::Equal => idx + 1,
                _ => idx,
            }
        } else {
            match this_key.cmp(key) {
                std::cmp::Ordering::Equal => self.find_partition_inner(key, low, idx),
                std::cmp::Ordering::Less => self.find_partition_inner(key, idx + 1, high),
                std::cmp::Ordering::Greater => {
                    if idx == 0 {
                        0
                    } else {
                        self.find_partition_inner(key, low, idx - 1)
                    }
                }
            }
        }
    }

    /// Determines whether the page has sufficient space for an insertion of `required` bytes.
    ///
    /// Distinguishes between two kinds of free space:
    ///
    /// - [`HeaderElem::ContFreeSpace`]: contiguous free space at the end of the data region.
    ///   An insert can proceed immediately if this is large enough.
    /// - [`HeaderElem::TotalFreeSpace`]: total free bytes including fragmented gaps left by
    ///   previous deletions. If only this is sufficient, the page must be
    ///   [collapsed](Self::collapse) first to compact the fragments into contiguous space.
    ///
    /// # Returns
    ///
    /// - [`SpaceStatus::Ok`] — enough contiguous space; insert can proceed directly.
    /// - [`SpaceStatus::NeedsCollapse`] — enough total space but fragmented; compact first.
    /// - [`SpaceStatus::OutOfSpace`] — not enough space even after compaction; page must be split.
    fn check_space(&self, required: usize) -> SpaceStatus {
        if self.get_header(&HeaderElem::ContFreeSpace) as usize >= required {
            SpaceStatus::Ok
        } else if self.get_header(&HeaderElem::TotalFreeSpace) as usize >= required {
            SpaceStatus::NeedsCollapse
        } else {
            SpaceStatus::OutOfSpace
        }
    }

    pub fn insert(&mut self, data: &Tuple) -> Result<(), PageReadWriteError> {
        // We need to check how much space we have.
        match self.check_space(data.len() + 2usize) {
            SpaceStatus::Ok => {}
            SpaceStatus::NeedsCollapse => self.collapse(),
            SpaceStatus::OutOfSpace => return Err(PageReadWriteError::OutOfSpace),
        }

        // Get slot write ptr and tuple write ptr
        let tuple_write_ptr = self.get_header(&HeaderElem::FreeSpacePtr) as usize - data.len();
        let slot_write_idx = self.find_partition(data.key());

        // I think the safest order is probably write out Tuple, then Slot, then update headers?
        // Write first
        // let mut_content = self.content_mut();
        self.0[tuple_write_ptr..tuple_write_ptr + data.len()].copy_from_slice(&data.0);
        self.write_slot_at(slot_write_idx, tuple_write_ptr.try_into().unwrap());

        // Update header
        self.update_header_insert(data.size().into());

        Ok(())
    }

    fn update_header_insert(&mut self, insert_size: i64) {
        use HeaderElem::*;
        self.update_header(&FreeSpacePtr, -insert_size);
        self.update_header(&ItemCount, 1);
        self.update_header(&ContFreeSpace, -insert_size - 2);
        self.update_header(&TotalFreeSpace, -insert_size - 2);
    }

    fn write_slot_at(&mut self, idx: usize, ptr: u16) {
        // Move everything over
        let item_count = self.get_header(&HeaderElem::ItemCount) as usize;
        let start_ptr = HEADER_SIZE + idx * 2;
        if item_count > 0 {
            let end_ptr = HEADER_SIZE + (item_count - 1) * 2;
            self.0.copy_within(start_ptr..end_ptr + 2, start_ptr + 2);
        }
        self.0[start_ptr..start_ptr + 2].copy_from_slice(&ptr.to_be_bytes());
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<(), PageReadWriteError> {
        let delete_idx = self
            .find_key(key)
            .ok_or(PageReadWriteError::KeyNotFound(format!("{:?}", key)))?;

        // Set new item count
        let current_count = self.get_header(&HeaderElem::ItemCount);
        self.set_header(&HeaderElem::ItemCount, current_count - 1);

        // Move everything over to cover now deleted key.
        // The `end_ptr` maps from current item count to copy end.
        // Ex: if 1 item, then the end_ptr will be 2.
        if current_count > 1 {
            let start_ptr = HEADER_SIZE + delete_idx * 2;
            let end_ptr = HEADER_SIZE + current_count as usize * 2;
            self.0.copy_within(start_ptr..end_ptr, start_ptr - 2);
        };
        Ok(())
    }

    pub fn split_half(&mut self) -> Vec<TupleBuf> {
        // Get how many items vs keep vs remove. Left is kept, right is moved.
        let item_count = self.get_header(&HeaderElem::ItemCount) as usize;
        let split_idx = item_count / 2;

        // Grab tuples that will go right
        let mut tuples: Vec<TupleBuf> = Vec::with_capacity(item_count);
        for idx in split_idx..item_count {
            tuples.push(self.tuple(idx).unwrap().to_owned());
        }

        // Only keep left side of the page
        self.keep_left(split_idx);

        // Return tuples
        tuples
    }

    /// Cleans up the page, only keeping the first 0..split_idx items in the page
    fn keep_left(&mut self, split_idx: usize) {
        // Create new temporary page to copy left into, then replace self.0
        let mut temp_bytes = [0u8; PAGE_SIZE];
        let temp = SlottedPage::from_bytes_mut(&mut temp_bytes);
        temp.init(
            self.get_header(&HeaderElem::PageID),
            self.get_header(&HeaderElem::PageType).try_into().unwrap(), // unwrap; cannot fail
            self.get_header(&HeaderElem::LeftSiblingPtr),
            self.get_header(&HeaderElem::RightSiblingPtr),
            Some(self.get_header(&HeaderElem::LeftChildPtr)),
        );
        for idx in 0..split_idx {
            temp.insert(self.tuple(idx).unwrap()).unwrap();
        }
        self.0[0..PAGE_SIZE].copy_from_slice(&temp.0[0..PAGE_SIZE]);
    }

    /// Collapse empty space in a page, so that [`HeaderElem::ContFreeSpace`]
    /// and [`HeaderElem::TotalFreeSpace`] match.
    pub fn collapse(&mut self) {
        self.keep_left(self.get_header(&HeaderElem::ItemCount) as usize);
    }
}

/// DST representing a leaf node.
///
/// Leaf node derefs right through to the slotted page.
#[repr(transparent)]
pub struct Leaf(SlottedPage);

impl Leaf {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const SlottedPage as *const Leaf) }
    }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        unsafe { &mut *(bytes as *mut [u8] as *mut SlottedPage as *mut Leaf) }
    }

    pub fn init(&mut self, page_id: PageID, left_ptr: PageID, right_ptr: PageID) {
        self.0
            .init(page_id, PageType::Leaf, left_ptr, right_ptr, None);
    }
}

impl Deref for Leaf {
    type Target = SlottedPage;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Leaf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// DST represetnign a inner node.
///
/// This is a special case of the leaf / slotted page,
/// and so we wrap methods.
#[repr(transparent)]
pub struct InnerNode(SlottedPage);

impl InnerNode {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const SlottedPage as *const InnerNode) }
    }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        unsafe { &mut *(bytes as *mut [u8] as *mut SlottedPage as *mut InnerNode) }
    }

    /// A wrapper of the base SlottedPage insert.
    ///
    /// Creates a new tuple from a page_id that will be the new child.
    pub fn insert(&mut self, key: &[u8], page_id: PageID) -> Result<(), PageReadWriteError> {
        let t = TupleBuf::new(key, &page_id.to_be_bytes());
        self.0.insert(&t)
    }

    pub fn init(
        &mut self,
        page_id: PageID,
        left_ptr: PageID,
        right_ptr: PageID,
        left_child_ptr: PageID,
    ) {
        self.0.init(
            page_id,
            PageType::Node,
            left_ptr,
            right_ptr,
            Some(left_child_ptr),
        );
    }

    /// Get the next child page in the search for `key`
    ///
    /// If we use base find partition, then we get an index back.
    /// We actually need to shift everything back one.
    ///
    /// Say we have twos keys (4, 7).
    /// - If `key = 2` => `left_child_ptr`  , but `find_partition` returns `0`
    /// - If `key = 4` => value at `idx = 0`, but `find_partition` returns `1`
    /// - If `key = 5` => value at `idx = 0`, but `find_partition` returns `1`
    /// - If `key = 7` => value at `idx = 1`, but `find_partition` returns `2`
    pub fn child(&self, key: &[u8]) -> PageID {
        let found_idx = self.0.find_partition(key);
        if found_idx == 0 {
            self.0.get_header(&HeaderElem::LeftChildPtr)
        } else {
            u32::from_be_bytes(
                self.0
                    .tuple(found_idx - 1)
                    .unwrap()
                    .value()
                    .try_into()
                    .unwrap(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helper: create an initialized InnerNode from a stack buffer
    // -----------------------------------------------------------------------
    fn make_inner_node(
        bytes: &mut [u8; PAGE_SIZE],
        page_id: PageID,
        left_child_ptr: PageID,
    ) -> &mut InnerNode {
        let node = InnerNode::from_bytes_mut(bytes);
        node.init(page_id, 0, 0, left_child_ptr);
        node
    }

    // -----------------------------------------------------------------------
    // InnerNode::child — Base Tests
    // -----------------------------------------------------------------------

    /// With keys [4, 7] and a key less than both, child() must return left_child_ptr.
    ///
    /// Tree picture:  left_child_ptr | 4 => page_a | 7 => page_b
    /// key = 2 falls left of everything => left_child_ptr (10).
    #[test]
    fn child_key_less_than_all_keys() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        node.insert(&[4u8], 20).unwrap();
        node.insert(&[7u8], 30).unwrap();

        assert_eq!(10, node.child(&[2u8]));
    }

    /// With keys [4, 7], key == 4 should follow the child pointer stored *with* key 4
    /// (page 20), not the left_child_ptr.
    ///
    /// B-tree convention: an inner-node key k means "all keys >= k go right".
    #[test]
    fn child_key_equal_to_first_key() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        node.insert(&[4u8], 20).unwrap();
        node.insert(&[7u8], 30).unwrap();

        assert_eq!(20, node.child(&[4u8]));
    }

    /// With keys [4, 7], key = 5 lies between the two keys.
    /// It belongs in the same child as key 4's right side (page 20).
    #[test]
    fn child_key_between_keys() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        node.insert(&[4u8], 20).unwrap();
        node.insert(&[7u8], 30).unwrap();

        assert_eq!(20, node.child(&[5u8]));
    }

    /// With keys [4, 7], key == 7 should return the child pointer stored with key 7 (page 30).
    #[test]
    fn child_key_equal_to_last_key() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        node.insert(&[4u8], 20).unwrap();
        node.insert(&[7u8], 30).unwrap();

        assert_eq!(30, node.child(&[7u8]));
    }

    /// With keys [4, 7], key = 9 is greater than all keys.
    /// It should still follow the rightmost child pointer (page 30, stored with key 7).
    #[test]
    fn child_key_greater_than_all_keys() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        node.insert(&[4u8], 20).unwrap();
        node.insert(&[7u8], 30).unwrap();

        assert_eq!(30, node.child(&[9u8]));
    }

    // -----------------------------------------------------------------------
    // InnerNode::child — Edge Case Tests
    // -----------------------------------------------------------------------

    /// An empty inner node has no separator keys. Any lookup must return left_child_ptr.
    #[test]
    fn child_empty_node_returns_left_child_ptr() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 99);

        // Regardless of key, the only known child is left_child_ptr.
        assert_eq!(99, node.child(&[0u8]));
        assert_eq!(99, node.child(&[255u8]));
    }

    /// Single key: lookup with key < the only separator returns left_child_ptr.
    #[test]
    fn child_single_key_less_than_separator() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        node.insert(&[5u8], 20).unwrap();

        assert_eq!(10, node.child(&[3u8]));
    }

    /// Single key: lookup with key == the separator returns the child stored with that key.
    #[test]
    fn child_single_key_equal_to_separator() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        node.insert(&[5u8], 20).unwrap();

        assert_eq!(20, node.child(&[5u8]));
    }

    /// Single key: lookup with key > the separator returns the child stored with that key.
    #[test]
    fn child_single_key_greater_than_separator() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        node.insert(&[5u8], 20).unwrap();

        assert_eq!(20, node.child(&[8u8]));
    }

    /// Multi-byte keys: verify routing still works correctly when keys are wider than one byte.
    #[test]
    fn child_multi_byte_key_routing() {
        let mut bytes = [0u8; PAGE_SIZE];
        let node = make_inner_node(&mut bytes, 1, 10);
        // Insert separator key [0, 100] => page 20
        node.insert(&[0u8, 100u8], 20).unwrap();

        // key [0, 50] is less than [0, 100] => left_child_ptr
        assert_eq!(10, node.child(&[0u8, 50u8]));
        // key [0, 100] equals separator => page 20
        assert_eq!(20, node.child(&[0u8, 100u8]));
        // key [0, 200] is greater => page 20
        assert_eq!(20, node.child(&[0u8, 200u8]));
    }

    // -----------------------------------------------------------------------
    // Existing tests
    // -----------------------------------------------------------------------

    #[test]
    fn new_page() {
        let mut bytes = vec![0u8; PAGE_SIZE];
        let page = Leaf::from_bytes_mut(&mut bytes);
        page.init(1, 15, 66);

        assert_eq!(1, page.get_header(&HeaderElem::PageID));
        assert_eq!(
            PageType::Leaf,
            PageType::new(page.get_header(&HeaderElem::PageType)).unwrap()
        );
    }

    #[test]
    fn tuple_insert() {
        let mut bytes = vec![0u8; PAGE_SIZE];
        let page = Leaf::from_bytes_mut(&mut bytes);
        page.init(1, 15, 66);

        // Insert tuple 1
        let tuple = TupleBuf::new(&[1u8], &[1u8]);
        page.insert(&tuple).unwrap();
        assert_eq!(&*tuple, page.tuple(0).unwrap());

        // Insert tuple 2. This should go before 1.
        let tuple = TupleBuf::new(&[0u8], &[2u8]);
        page.insert(&tuple).unwrap();
        assert_eq!(&*tuple, page.tuple(0).unwrap());

        // Delete tuple 2. This should move 1 back to 0.
        page.delete(&[0u8]).unwrap();
        let tuple = TupleBuf::new(&[1u8], &[1u8]);
        assert_eq!(&*tuple, page.tuple(0).unwrap());
    }
}
