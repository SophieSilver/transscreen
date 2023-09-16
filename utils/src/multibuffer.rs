use std::{
    mem,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use parking_lot::RwLock;

/// A data structure that contains a locally stored back buffer for editing
/// as well as a shared front buffer for access in other parts of the code.
///
/// # Swapping
/// Buffers can be swapped, which will push the data from the back buffer
/// into the front buffer, where it can be accessed by other holders of the MultiBuffer,
/// at the same time the data currently stored in the front buffer is pulled into the back buffer.
///
/// # Cloning
/// Cloning the `MultiBuffer` Clones the current back buffer, but keeps the reference to the front buffer the same
#[derive(Debug, Clone)]
pub struct MultiBuffer<T> {
    back: T,
    front: Arc<RwLock<T>>,
}

impl<T> MultiBuffer<T> {
    /// Constructs a new `MultiBuffer`. `val` will be cloned to create both the front and back buffers.
    /// In case `T` doesn't implement `Clone`, `from_buffers` should be used instead.
    #[inline]
    pub fn new(val: T) -> Self
    where
        T: Clone,
    {
        let front = val;
        let back = Arc::new(RwLock::new(front.clone()));

        Self { back: front, front: back }
    }

    /// constructs the `MultiBuffer` out of two different buffers.
    ///
    /// Can be used in case `T` doesn't implement `Clone`.
    #[inline]
    pub fn from_buffers(front: T, back: T) -> Self {
        let back = Arc::new(RwLock::new(back));
        
        Self { back: front, front: back }
    }
    
    /// Swaps the front and back buffers. 
    /// 
    /// This gives the holder of this struct 
    /// exclusive ownerwhip of the data in the back buffer, 
    /// while making the current front buffer accessible to other `MultiBuffer`s pointing to the same buffer.
    /// 
    /// May block if the front buffer is currently locked.
    #[inline]
    pub fn swap(&mut self) {
        let front = &mut *self.front.write();

        mem::swap(&mut self.back, front);
    }
    
    /// Swaps the front and back buffers. 
    /// 
    /// This gives the holder of this struct 
    /// exclusive ownerwhip of the data in the back buffer, 
    /// while making the current front buffer accessible to other `MultiBuffer`s pointing to the same back buffer.
    /// 
    /// returns `None` if the front buffer is currently locked
    #[inline]
    #[must_use]
    pub fn try_swap(&mut self) -> Option<()> {
        let front = &mut *self.front.try_write()?;

        mem::swap(&mut self.back, front);
        
        Some(())
    }
    
    /// Allows to clone this `MultiBuffer` by providing a new back buffer
    /// in case `T` doesn't implement Clone
    #[inline]
    pub fn clone_with_back(&self, new_back: T) -> Self {
        Self {
            back: new_back,
            front: self.front.clone(),
        }
    }
    
    #[inline]
    pub fn back(&self) -> &T {
        &self.back
    }

    #[inline]
    pub fn back_mut(&mut self) -> &mut T {
        &mut self.back
    }
    
    #[inline]
    pub fn view(&self) -> MultiBufferView<T> {
        MultiBufferView { front: self.front.clone() }
    }
    
    /// Returns a reference-like object to the front buffer.
    /// 
    /// When using this method, as well as `front_mut`, keep in mind that
    /// it would generally lock the front buffer for as long as the reference is held. 
    /// This would block the incoming calls to `swap`.
    /// 
    /// The return type is a lock guard, but the exact type is implementation defined and can change.
    /// As such, trying to get the front buffer while a reference to it already exists in the current thread, might
    /// result in a deadlock
    #[inline]
    pub fn front(&self) -> impl Deref<Target = T> + '_ {
        self.front.read()
    }

    /// Returns a mutable reference-like object to the front buffer.
    /// 
    /// When using this method, as well as `front`, keep in mind that
    /// it would generally lock the front buffer for as long as the reference is held. 
    /// This would block the incoming calls to `swap`.
    /// 
    /// The returned value is a lock guard, but the exact type is implementation defined and can change.
    /// As such, trying to get the front buffer while a reference to it already exists in the current thread, might
    /// result in a deadlock
    #[inline]
    pub fn front_mut(&self) -> impl DerefMut<Target = T> + '_ {
        self.front.write()
    }
    
    /// Returns a reference-like object to the front buffer.
    /// 
    /// Same as `front`, except it will return None if the operation would block.
    #[inline]
    pub fn try_front(&self) -> Option<impl Deref<Target = T> + '_> {
        self.front.try_read()
    }

    /// Returns a mutable reference-like object to the front buffer.
    /// 
    /// Same as `front_mut`, except it will return None if the operation would block.
    #[inline]
    pub fn try_front_mut(&self) -> Option<impl DerefMut<Target = T> + '_> {
        self.front.try_write()
    }
}


impl<T> Deref for MultiBuffer<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.back()
    }
}

impl<T> DerefMut for MultiBuffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.back_mut()
    }
}


/// Similar to `MultiBuffer` but only has access to the front buffer
#[derive(Clone)]
pub struct MultiBufferView<T> {
    front: Arc<RwLock<T>>,
}

impl<T> MultiBufferView<T> {
    /// Returns a reference-like object to the front buffer.
    /// 
    /// When using this method, as well as `front_mut`, keep in mind that
    /// it would generally lock the front buffer for as long as the reference is held. 
    /// This would block the incoming calls to `swap`.
    /// 
    /// The return type is a lock guard, but the exact type is implementation defined and can change.
    /// As such, trying to get the front buffer while a reference to it already exists in the current thread, might
    /// result in a deadlock
    #[inline]
    pub fn front(&self) -> impl Deref<Target = T> + '_ {
        self.front.read()
    }
    
    /// Returns a reference-like object to the front buffer.
    /// 
    /// Same as `front`, except it will return None if the operation would block.
    #[inline]
    pub fn try_front(&self) -> Option<impl Deref<Target = T> + '_> {
        self.front.try_read()
    }
}