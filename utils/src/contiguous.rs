use std::{
    collections::VecDeque,
    io::Write,
};

use thiserror::Error;

/// Used for defining data chunks' boundaries in contiguous buffers as well as its metadata
#[derive(Debug, Clone, Copy)]
struct ItemData<M> {
    start_index: usize,
    length: usize,
    metadata: M,
}

#[derive(Debug, Clone, Copy)]
pub struct BufferItem<'a, M> {
    data: &'a [u8],
    metadata: &'a M,
}

impl<'a, M> BufferItem<'a, M> {
    #[inline]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    #[inline]
    pub fn metadata(&self) -> &M {
        self.metadata
    }
}

/// A Ring buffer holding arbitrary sized byte chunks contiguously.
#[derive(Debug, Clone)]
pub struct RingBuffer<M> {
    buf: Box<[u8]>,
    items: VecDeque<ItemData<M>>,

    write_head_position: usize,
    // used for preserving indices even after overwriting elements 
    // and popping items from the front of the queue
    id_offset: usize
    // max id is just id_offset + items.len()
}

impl<M> RingBuffer<M> {
    #[inline]
    pub fn new(cap: usize) -> Self {
        let buf = vec![0; cap].into_boxed_slice();
        let items = VecDeque::new();

        Self {
            buf,
            items,
            write_head_position: 0,
            id_offset: 0,
        }
    }

    pub fn write(&mut self, data: &[u8], metadata: M) -> Result<(), WriteDataError> {
        if data.len() > self.buf.len() {
            return Err(WriteDataError::DataTooLarge);
        }

        // reset the write head if there isn't enough space in front of it
        let free_space = self.buf.len() - self.write_head_position;
        if free_space < data.len() {
            self.write_head_position = 0;
        }

        // write the data at head position
        let start_index = self.write_head_position;
        let end_index = start_index + data.len();

        let mut write_slice = &mut self.buf[start_index..end_index];
        // Safety: cannot fail since we've done the bounds check already
        write_slice.write_all(data).unwrap();

        self.write_head_position = end_index;

        // invalidate any overwritten items
        for _ in 0..self.items.len() {
            let other_item = match self.items.front() {
                Some(item) => item,
                None => break,
            };
            
            let other_item_end = other_item.start_index + other_item.length;

            if other_item.start_index < end_index && other_item_end > start_index {
                self.items.pop_front().unwrap();
                self.id_offset = self.id_offset.checked_add(1).expect("DataRingBuffer ids overflowed");
            } else {
                break;
            }
        }

        // register the new data chunk in the item deque
        let new_item =  ItemData {
                start_index,
                length: data.len(),
                metadata,
            };
        
        self.items.push_back(new_item);

        Ok(())
    }
    
    pub fn get(&self, id: usize) -> Option<BufferItem<M>> {
        let end = self.id_offset + self.items.len();
        // bounds check
        if id < self.id_offset || id >= end {
            // dbg!(self.index_offset);
            // dbg!(end);
            return None;
        }
        
        let index = id - self.id_offset;
        let item_data = &self.items[index];
        
        let slice_start = item_data.start_index;
        let slice_end = slice_start + item_data.length;
        
        let item = BufferItem {
            data: &self.buf[slice_start..slice_end],
            metadata: &item_data.metadata,
        };
        
        Some(item)
    }
    
    #[inline]
    pub fn id_bounds(&self) -> (usize, usize) {
        let min = self.id_offset;
        let max = self.id_offset + self.items.len();
        
        (min, max)
    }
    
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = BufferItem<M>>{
        Iter {
            buf: &self.buf,
            items: self.items.iter(),
        }
    }
    
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }
    
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Error)]
pub enum WriteDataError {
    #[error("data too large")]
    DataTooLarge,
}

#[derive(Debug, Clone, Default)]
pub struct GrowableBuffer<M> {
    buf: Vec<u8>,
    items: Vec<ItemData<M>>,
}

impl<M> GrowableBuffer<M> {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            items: Vec::new(),
        }
    }
    
    pub fn write(&mut self, data: &[u8], metadata: M) {
        let start_index = self.buf.len();
        let length = data.len();
        
        let item = ItemData {
            start_index,
            length,
            metadata,
        };
        
        self.buf.extend_from_slice(data);
        self.items.push(item);
    }
    
    pub fn dump_into_ring_buffer(&mut self, ring_buf: &mut RingBuffer<M>) -> Result<(), WriteDataError> {
        for item in self.items.drain(..) {
            let end_index = item.start_index + item.length;
            let data = &self.buf[item.start_index..end_index];
            
            ring_buf.write(data, item.metadata)?;
        }
        
        self.buf.clear();
        
        Ok(())
    }
    
    #[inline]
    pub fn get(&self, index: usize) -> Option<BufferItem<M>> {
        let item = self.items.get(index)?;
        let end = item.start_index + item.length;
        
        Some(BufferItem {
            data: &self.buf[item.start_index..end],
            metadata: &item.metadata,
        })
    }
    
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = BufferItem<M>> {
        Iter {
            buf: &self.buf,
            items: self.items.iter(),
        }
    }
    
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }
    
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

struct Iter<'a, M, I>
where
    M: 'a,
    I: Iterator<Item = &'a ItemData<M>>,
{
    buf: &'a [u8],
    items: I
}

impl<'a, M, I> Iterator for Iter<'a, M, I>
where
    M: 'a,
    I: Iterator<Item = &'a ItemData<M>>,
{
    type Item = BufferItem<'a, M>;

    fn next(&mut self) -> Option<Self::Item> {
        let next_item = self.items.next()?;
        let end = next_item.start_index + next_item.length;
        
        let data = &self.buf[next_item.start_index..end];
        
        Some(BufferItem {
            data,
            metadata: &next_item.metadata,
        })
    }
    
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn ring_buffer_add() {
        let chunk: &[u8] = &[1, 2, 3];
        
        let mut rb = RingBuffer::new(10);
        rb.write(chunk, ()).unwrap();
        
        assert_eq!(rb.get(0).unwrap().data, chunk);
    }
    
    #[test]
    fn ring_buffer_add_2() {
        let chunk1: &[u8] = &[1, 2, 3];
        let chunk2: &[u8] = &[4, 5, 6, 7, 8, 9, 10];
        
        let mut rb = RingBuffer::new(10);
        rb.write(chunk1, ()).unwrap();
        rb.write(chunk2, ()).unwrap();
        
        assert_eq!(rb.get(0).unwrap().data, chunk1);
        assert_eq!(rb.get(1).unwrap().data, chunk2);
    }
    
    #[test]
    fn ring_buffer_override() {
        let chunk1: &[u8] = &[1, 2, 3];
        let chunk2: &[u8] = &[4, 5, 6, 7, 8, 9, 10];
        
        let mut rb = RingBuffer::new(10);
        rb.write(chunk1, ()).unwrap();
        rb.write(chunk2, ()).unwrap();
        rb.write(chunk1, ()).unwrap();
        
        assert!(rb.get(0).is_none());
        assert_eq!(rb.get(1).unwrap().data, chunk2);
        assert_eq!(rb.get(2).unwrap().data, chunk1);
    }
    
    #[test]
    fn ring_buffer_override_2() {
        let chunk1: &[u8] = &[1, 2, 3];
        let chunk2: &[u8] = &[4, 5, 6, 7, 8, 9, 10];
        
        let mut rb = RingBuffer::new(10);
        rb.write(chunk1, ()).unwrap();
        rb.write(chunk2, ()).unwrap();
        rb.write(chunk2, ()).unwrap();
        
        assert!(rb.get(0).is_none());
        assert!(rb.get(1).is_none());
        assert_eq!(rb.get(2).unwrap().data, chunk2);
    }
    
    #[test]
    fn ring_buffer_iter() {
        let chunk1: &[u8] = &[1, 2, 3];
        let chunk2: &[u8] = &[4, 5, 6, 7, 8, 9, 10];
        
        let mut rb = RingBuffer::new(10);
        rb.write(chunk1, ()).unwrap();
        rb.write(chunk2, ()).unwrap();
        rb.write(chunk1, ()).unwrap();
        
        let mut iter = rb.iter();
        
        assert_eq!(iter.next().unwrap().data(), chunk2);
        assert_eq!(iter.next().unwrap().data(), chunk1);
        assert!(iter.next().is_none());
    }
    
    #[test]
    fn ring_buffer_bounds() {
        let chunk1: &[u8] = &[1, 2, 3];
        let chunk2: &[u8] = &[4, 5, 6, 7, 8, 9, 10];
        
        let mut rb = RingBuffer::new(10);
        rb.write(chunk1, ()).unwrap();
        rb.write(chunk2, ()).unwrap();
        
        let bounds = rb.id_bounds();
        assert_eq!(bounds, (0, 2));
    }
    
    #[test]
    fn ring_buffer_bounds_2() {
        let chunk: &[u8] = &[1, 2, 3];
        
        let mut rb = RingBuffer::new(11);
        
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        rb.write(chunk, ()).unwrap();
        
        let bounds = rb.id_bounds();
        
        assert!(rb.get(bounds.0).is_some());
        assert!(rb.get(bounds.1 - 1).is_some());
        assert!(rb.get(bounds.0 - 1).is_none());
        assert!(rb.get(bounds.1).is_none());
    }
    
    #[test]
    fn growable_dump() {
        let chunk: &[u8] = &[1, 2, 3];
        
        let mut rb = RingBuffer::new(24);
        
        let mut gb = GrowableBuffer::new();
        
        (0..8).for_each(|_| {
            gb.write(chunk, ());
        });
        
        gb.dump_into_ring_buffer(&mut rb).unwrap();
        
        let (min, max) = rb.id_bounds();
        
        (min..max).for_each(|id| {
            assert_eq!(rb.get(id).unwrap().data(), &[1, 2, 3]);
        });
    }
    
    #[test]
    fn ring_buffer_iter_2() {
        let chunk: &[u8] = &[1, 2, 3];
        
        let mut rb = RingBuffer::new(24);
        
        let mut gb = GrowableBuffer::new();
        
        (0..8).for_each(|_| {
            gb.write(chunk, ());
        });
        
        gb.dump_into_ring_buffer(&mut rb).unwrap();
        
        for i in rb.iter() {
            assert_eq!(i.data(), &[1, 2, 3]);
        }
    }
    
    #[test]
    fn growable_buffer_iter() {
        let chunk: &[u8] = &[1, 2, 3, 4, 5];
        
        let mut gb = GrowableBuffer::new();
        
        gb.write(chunk, ());
        gb.write(chunk, ());
        gb.write(chunk, ());
        gb.write(chunk, ());
        gb.write(chunk, ());
        gb.write(chunk, ());
        gb.write(chunk, ());
        gb.write(chunk, ());
        
        for i in gb.iter() {
            assert_eq!(i.data(), chunk);
        }
    }
}