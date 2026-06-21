use std::ops::{Deref, DerefMut};

use super::tuple::*;
use crate::buffer_pool::{PAGE_SIZE, PageID};

#[derive(Debug, thiserror::Error)]
pub enum PageWriteError {
    #[error("Page is out of space for insert")]
    OutOfSpace,
    #[error("Cannot find specified key (bytes: {0})")]
    KeyNotFound(String),
}

pub enum HeaderElem {
    PageID,
    PageType,
    FreeSpace,
    LastCommit,
    ItemCount,
    LeftPtr,
    RightPtr,
    FreeSpacePtr,
    LeftChildPtr,
}

impl HeaderElem {
    fn offset(&self) -> usize {
        match self {
            Self::PageID => 0,
            Self::PageType => 4,
            Self::FreeSpace => 8,
            Self::LastCommit => 12,
            Self::ItemCount => 16,
            Self::LeftPtr => 20,
            Self::RightPtr => 24,
            Self::FreeSpacePtr => 28,
            Self::LeftChildPtr => 32,
        }
    }
}

pub fn get_page_type(bytes: &[u8]) -> PageType {
    let offset = HeaderElem::PageType.offset();
    let type_id = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
    PageType::new(type_id)
}

const HEADER_SIZE: usize = 4 * 9;

/// Page type in the BTree. Node pages map keys to page ids, leaf pages map keys to tuples
#[derive(Debug, PartialEq)]
pub enum PageType {
    /// Nodes always map to Key => PageID
    Node,
    /// Leaf always maps Key => Tuple
    Leaf,
}

impl PageType {
    pub fn id(&self) -> u64 {
        match self {
            Self::Node => 0,
            Self::Leaf => 1,
        }
    }

    pub fn new(type_id: u32) -> Self {
        match type_id {
            0 => Self::Node,
            1 => Self::Leaf,
            _ => panic!(),
        }
    }
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

    pub fn get_header(&self, element: HeaderElem) -> u32 {
        let offset = element.offset();
        u32::from_be_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }

    pub fn set_header(&mut self, element: HeaderElem, value: u32) {
        let offset = element.offset();
        self.0[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    pub fn init(
        &mut self,
        page_id: PageID,
        page_type: PageType,
        left_ptr: PageID,
        right_ptr: PageID,
        left_child_ptr: Option<PageID>,
    ) {
        self.set_header(HeaderElem::PageID, page_id.try_into().unwrap());
        self.set_header(HeaderElem::PageType, page_type.id().try_into().unwrap());
        self.set_header(
            HeaderElem::FreeSpace,
            (PAGE_SIZE - HEADER_SIZE).try_into().unwrap(),
        ); // 2 bytes for first slot
        self.set_header(HeaderElem::ItemCount, 0);
        self.set_header(HeaderElem::LeftPtr, left_ptr.try_into().unwrap());
        self.set_header(HeaderElem::RightPtr, right_ptr.try_into().unwrap());
        self.set_header(HeaderElem::FreeSpacePtr, PAGE_SIZE.try_into().unwrap());

        if let Some(x) = left_child_ptr {
            self.set_header(HeaderElem::LeftChildPtr, x.try_into().unwrap())
        };
    }

    pub fn tuple(&self, idx: usize) -> Option<&Tuple> {
        // Read first u16 / get size
        let ptr = if self.get_header(HeaderElem::ItemCount) <= idx as u32 {
            None
        } else {
            let slot_ptr = HEADER_SIZE + idx * 2;
            Some(u16::from_be_bytes(self.0[slot_ptr..slot_ptr + 2].try_into().unwrap()) as usize)
        }?;
        let tuple_size = u16::from_be_bytes(self.0[ptr..ptr + 2].try_into().unwrap()) as usize;
        Some(Tuple::from_bytes(&self.0[ptr..ptr + tuple_size]))
    }

    pub fn find_key(&self, key: &[u8]) -> Option<usize> {
        let count = self.get_header(HeaderElem::ItemCount);
        if count == 0 {
            None
        } else {
            self.find_key_inner(key, 0, self.get_header(HeaderElem::ItemCount) as usize - 1)
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
        let count = self.get_header(HeaderElem::ItemCount);
        if count == 0 {
            0
        } else {
            self.find_partition_inner(key, 0, self.get_header(HeaderElem::ItemCount) as usize - 1)
        }
    }

    fn find_partition_inner(&self, key: &[u8], low: usize, high: usize) -> usize {
        let idx = low + (high - low) / 2;

        let this_key = self.tuple(idx).unwrap().key();
        if high == low {
            match this_key.cmp(key) {
                std::cmp::Ordering::Less => idx + 1,
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

    pub fn insert(&mut self, data: &Tuple) -> Result<(), PageWriteError> {
        // First, check free space
        let free_space = self.get_header(HeaderElem::FreeSpace);
        if (data.len() + 2usize) > free_space as usize {
            return Err(PageWriteError::OutOfSpace);
        }

        // Get slot write ptr and tuple write ptr
        let tuple_write_ptr = self.get_header(HeaderElem::FreeSpacePtr) as usize - data.len();

        // Find slot write idx
        let slot_write_idx = self.find_partition(data.key());

        // I think the safest order is probably write out Tuple, then Slot, then update headers?
        // Write first
        // let mut_content = self.content_mut();
        self.0[tuple_write_ptr..tuple_write_ptr + data.len()].copy_from_slice(&data.0);
        self.write_slot_at(slot_write_idx, tuple_write_ptr.try_into().unwrap());

        // Update header
        let item_count = self.get_header(HeaderElem::ItemCount);
        self.set_header(HeaderElem::FreeSpace, free_space - data.len() as u32);
        self.set_header(HeaderElem::FreeSpacePtr, tuple_write_ptr as u32);
        self.set_header(HeaderElem::ItemCount, item_count + 1);

        Ok(())
    }

    fn write_slot_at(&mut self, idx: usize, ptr: u16) {
        // Move everything over
        let item_count = self.get_header(HeaderElem::ItemCount) as usize;
        let start_ptr = HEADER_SIZE + idx * 2;
        if item_count > 0 {
            let end_ptr = HEADER_SIZE + (item_count - 1) * 2;
            self.0.copy_within(start_ptr..end_ptr + 2, start_ptr + 2);
        }
        self.0[start_ptr..start_ptr + 2].copy_from_slice(&ptr.to_be_bytes());
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<(), PageWriteError> {
        let delete_idx = self
            .find_key(key)
            .ok_or(PageWriteError::KeyNotFound(format!("{:?}", key)))?;

        // Set new item count
        let current_count = self.get_header(HeaderElem::ItemCount);
        self.set_header(HeaderElem::ItemCount, current_count - 1);

        // Move everything over to cover now deleted key
        if current_count > 1 {
            let start_ptr = HEADER_SIZE + delete_idx * 2;
            let end_ptr = HEADER_SIZE + current_count as usize * 2;
            self.0.copy_within(start_ptr..end_ptr + 2, start_ptr - 2);
        };
        Ok(())
    }

    pub fn split_half(&mut self) -> Vec<TupleBuf> {
        // Get how many items vs keep vs remove. Left is kept, right is moved.
        let item_count = self.get_header(HeaderElem::ItemCount) as usize;
        let split_idx = item_count / 2;

        // Grab tuples that will go right
        let mut tuples: Vec<TupleBuf> = Vec::new();
        for idx in split_idx..item_count {
            tuples.push(self.tuple(idx).unwrap().to_owned());
        }

        // Create new temporary page to copy left into, then replace self.0
        let mut temp_bytes = [0u8; PAGE_SIZE];
        let temp = SlottedPage::from_bytes_mut(&mut temp_bytes);
        for idx in 0..split_idx {
            temp.insert(self.tuple(idx).unwrap()).unwrap();
        }
        self.0[HEADER_SIZE..PAGE_SIZE].copy_from_slice(&temp.0[HEADER_SIZE..PAGE_SIZE]);

        // Return tuples
        tuples
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

    pub fn init(
        &mut self,
        page_id: PageID,
        left_ptr: PageID,
        right_ptr: PageID,
    ) {
        self.0.init(page_id, PageType::Leaf, left_ptr, right_ptr, None);
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

    pub fn insert(&mut self, key: &[u8], page_id: PageID) -> Result<(), PageWriteError> {
        // construct tuple
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_page() {
        let mut bytes = vec![0u8; PAGE_SIZE];
        let page = Leaf::from_bytes_mut(&mut bytes);
        page.init(1, 15, 66);

        assert_eq!(1, page.get_header(HeaderElem::PageID));
        assert_eq!(
            PageType::Leaf,
            PageType::new(page.get_header(HeaderElem::PageType))
        );
    }

    #[test]
    fn tuple_insert() {
        let mut bytes = vec![0u8; PAGE_SIZE];
        let page = Leaf::from_bytes_mut(&mut bytes);
        page.init(1,  15, 66);

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

    // #[test]
    // fn page_split() {
    //     for i in 0..20 as u8 {
    //         let tuple = TupleBuf::new(&[i], &[i]);
    //         page.insert(&tuple).unwrap();
    //     }
    // }
}
