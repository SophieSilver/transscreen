use std::{sync::Arc, ops::Deref};

use parking_lot::{RwLock, RwLockReadGuard};
use utils::contiguous::{RingBuffer, GrowableBuffer, self};

#[derive(Debug)]
pub struct Metadata {
    pub is_key: bool,
}

#[derive(Debug)]
pub struct EncodedBuffer {
    ring_buf: Arc<RwLock<RingBuffer<Metadata>>>,
    write_buf: GrowableBuffer<Metadata>,
}

impl EncodedBuffer {
    pub fn new(capacity: usize) -> Self {
        let ring_buf = RingBuffer::new(capacity);
        let ring_buf = Arc::new(RwLock::new(ring_buf));
        
        let write_buf = GrowableBuffer::new();
        
        Self {
            ring_buf,
            write_buf,
        }
    }
    
    pub fn write(&mut self, data: &[u8], metadata: Metadata) {
        self.write_buf.write(data, metadata);
    }
    
    pub fn write_flush(&mut self, data: &[u8], metadata: Metadata) -> Result<(), contiguous::WriteDataError> {
        self.flush()?;
        self.ring_buf.write().write(data, metadata)?;
        
        Ok(())
    }
    
    pub fn flush(&mut self)  -> Result<(), contiguous::WriteDataError> {
        self.write_buf.dump_into_ring_buffer(&mut self.ring_buf.write())
    }
    
    pub fn view(&self) -> EncodedBufferView {
        let buf = self.ring_buf.clone();
        EncodedBufferView { buf }
    }
    
    pub fn write_buf_len(&self) -> usize {
        self.write_buf.len()
    }
    
    pub fn write_buf_is_empty(&self) -> bool {
        self.write_buf.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct EncodedBufferView {
    buf: Arc<RwLock<RingBuffer<Metadata>>>,
}

impl EncodedBufferView {
    pub fn get(&self) -> EncodedDataGuard<'_> {
        EncodedDataGuard { inner: self.buf.read() }
    }
}

pub struct EncodedDataGuard<'a> {
    inner: RwLockReadGuard<'a, RingBuffer<Metadata>>,
}

impl Deref for EncodedDataGuard<'_> {
    type Target = RingBuffer<Metadata>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}