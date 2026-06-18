use crate::reader::HeaderElem::FreeSpace;

use super::buffer_pool::{PAGE_SIZE, PageID};
use std::{cmp::Ordering::Less, io::Read, ops::Deref, slice::SliceIndex};

/// DST representing a tuple in data.
///
/// This has the following format:
///
/// \[Header(size: u16) | Key(size: u16, *bytes) | Value(*bytes) ]
#[repr(transparent)]
#[derive(Debug, PartialEq)]
pub struct Tuple([u8]);

impl Tuple {
    fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const Tuple) }
    }

    fn size(&self) -> u16 {
        u16::from_be_bytes(self.0[0..2].try_into().unwrap())
    }

    fn key_size(&self) -> u16 {
        u16::from_be_bytes(self.0[2..4].try_into().unwrap())
    }

    fn key(&self) -> &[u8] {
        let key_size = self.key_size();
        &self.0[4..4 + key_size as usize]
    }

    fn value(&self) -> &[u8] {
        let key_size = self.key_size() as usize;
        &self.0[4 + key_size..self.0.len()]
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::fmt::Display for Tuple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[ {:x} | [ {:x} | {:?} ] | [ {:x?} ] ]",
            self.size(),
            self.key_size(),
            self.key(),
            self.value()
        )
    }
}

pub struct TupleBuf {
    bytes: Vec<u8>,
}

impl TupleBuf {
    fn new(key: &[u8], value: &[u8]) -> Self {
        let mut data = Vec::<u8>::new();
        let size = key.len() + value.len() + 4; // add key size + value size + total tuple size (tuple + key headers)
        data.extend_from_slice((size as u16).to_be_bytes().as_slice());
        data.extend_from_slice((key.len() as u16).to_be_bytes().as_slice());
        data.extend_from_slice(key);
        data.extend_from_slice(value);
        TupleBuf { bytes: data }
    }
}

impl Deref for TupleBuf {
    type Target = Tuple;

    fn deref(&self) -> &Self::Target {
        Tuple::from_bytes(&self.bytes)
    }
}

impl std::fmt::Display for TupleBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

// impl DerefMut for TupleBuf {

//     fn deref_mut(&self) -> &mut Self::Target {
//         &mut Tuple::from_(&self.bytes)
//     }
// }

/// DST representing a page in data.
///
/// Has a header and content.
#[repr(transparent)]
pub struct Page([u8]);

impl Page {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const Self) }
    }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        unsafe { &mut *(bytes as *mut [u8] as *mut Self) }
    }
}

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

pub enum HeaderElem {
    PageID,
    PageType,
    FreeSpace,
    LastCommit,
    ItemCount,
    LeftPtr,
    RightPtr,
    FreeSpacePtr,
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
        }
    }
}

const HEADER_SIZE: usize = 4 * 8;

#[derive(Debug, thiserror::Error)]
pub enum PageWriteError {
    #[error("Page is out of space for insert")]
    OutOfSpace,
}

pub trait PageReader {
    fn get_header(&self, element: HeaderElem) -> u32;
    fn tuple(&self, idx: usize) -> Option<&Tuple>;

    /// Find a key and return the slot idx
    fn find_key(&self, key: &[u8]) -> Option<usize>;
    fn find_key_inner(&self, key: &[u8], low: usize, high: usize) -> Option<usize>;

    /// Find where to insert a new key
    fn find_partition(&self, key: &[u8]) -> usize;
    fn find_partition_inner(&self, key: &[u8], low: usize, high: usize) -> usize;

    /// This is only for inner nodes
    fn find_child(&self, key: &[u8]) -> PageID;
}

pub trait PageWriter {
    fn set_header(&mut self, element: HeaderElem, value: u32);
    fn init(&mut self, page_id: PageID, page_type: PageType, left_ptr: PageID, right_ptr: PageID);
    fn insert(&mut self, data: &Tuple) -> Result<(), PageWriteError>;
    fn write_slot_at(&mut self, idx: usize, ptr: u16);
}

impl PageReader for Page {
    fn get_header(&self, element: HeaderElem) -> u32 {
        let offset = element.offset();
        u32::from_be_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }

    fn tuple(&self, idx: usize) -> Option<&Tuple> {
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

    fn find_key(&self, key: &[u8]) -> Option<usize> {
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

    /// Just iterate through page
    fn find_child(&self, key: &[u8]) -> PageID {
        let mut ptr = 0usize;
        let mut idx = 0;
        let items = self.get_header(HeaderElem::ItemCount);
        loop {
            // read page id
            let page_id = u64::from_be_bytes(self.0[ptr..ptr + 8].try_into().unwrap());
            idx += 1;

            // end of the line
            if idx >= items {
                break page_id;
            }
            ptr += 8;

            // read key length
            let length = u16::from_be_bytes(self.0[ptr..ptr + 2].try_into().unwrap()) as usize;
            ptr += 2;

            // read and compare key. If next key is greater, then break with last page id
            if self.0[ptr..ptr + length] > *key {
                break page_id;
            }
            ptr += length;
        }
    }

    fn find_partition(&self, key: &[u8]) -> usize {
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
}

impl PageWriter for Page {
    fn init(&mut self, page_id: PageID, page_type: PageType, left_ptr: PageID, right_ptr: PageID) {
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
    }

    fn insert(&mut self, data: &Tuple) -> Result<(), PageWriteError> {
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

    fn set_header(&mut self, element: HeaderElem, value: u32) {
        let offset = element.offset();
        self.0[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use crate::serialization::{DataType, DataValue, Serializer};

    use super::*;

    #[test]
    fn new_page() {
        let mut bytes = vec![0u8; PAGE_SIZE];
        let page = Page::from_bytes_mut(&mut bytes);
        page.init(1, PageType::Leaf, 15, 66);

        assert_eq!(1, page.get_header(HeaderElem::PageID));
        assert_eq!(
            PageType::Leaf,
            PageType::new(page.get_header(HeaderElem::PageType))
        );
    }

    #[test]
    fn tuple_insert() {
        let mut bytes = vec![0u8; PAGE_SIZE];
        let page = Page::from_bytes_mut(&mut bytes);
        page.init(1, PageType::Leaf, 15, 66);

        // Insert tuple 1
        let key = Serializer::serialize_single(&DataType::Int, &DataValue::Int(5)).unwrap();
        let value = Serializer::serialize_single(&DataType::Int, &DataValue::Int(15)).unwrap();
        let tuple = TupleBuf::new(&key, &value);
        assert!(page.insert(&tuple).is_ok());

        // Assert looks good
        let ret_tuple = page.tuple(0).unwrap();
        assert_eq!(&*tuple, ret_tuple);

        // Insert tuple 2; should go before 1
        let key = Serializer::serialize_single(&DataType::Int, &DataValue::Int(3)).unwrap();
        let value = Serializer::serialize_single(&DataType::Int, &DataValue::Int(15)).unwrap();
        let tuple = TupleBuf::new(&key, &value);
        assert!(page.insert(&tuple).is_ok());

        // Assert looks good
        let ret_tuple = page.tuple(0).unwrap();
        assert_eq!(&*tuple, ret_tuple);

        // Insert tuple 3; should go after 2
        let key = Serializer::serialize_single(&DataType::Int, &DataValue::Int(100)).unwrap();
        let value = Serializer::serialize_single(&DataType::Int, &DataValue::Int(15)).unwrap();
        let tuple = TupleBuf::new(&key, &value);
        assert!(page.insert(&tuple).is_ok());

        // Assert looks good
        let ret_tuple = page.tuple(2).unwrap();
        assert_eq!(&*tuple, ret_tuple)
    }
}
