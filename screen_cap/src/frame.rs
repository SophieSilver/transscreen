use std::{ops::Deref, io::{self, ErrorKind}};

use thiserror::Error;

/// A convenience type to go from "something that derefs into something else that derefs into `[u8]`"
/// into just something that derefs into `[u8]`.
/// 
/// usually used to abstract stuff like `LockGuard<Vec<u8>>` or `LockGuard<Box<u8>>` 
/// into just an `impl Deref<Target = [u8]>`
pub struct FrameGuard<T, U>
where
    T: Deref<Target = U>,
    U: Deref<Target = [u8]>
{
    guard: T,
}

impl<T, U> FrameGuard<T, U> 
where
    T: Deref<Target = U>,
    U: Deref<Target = [u8]>
{
    #[inline]
    pub fn new(guard: T) -> Self {
        Self {guard}
    }
}

impl<T, U> Deref for FrameGuard<T, U> 
where
    T: Deref<Target = U>,
    U: Deref<Target = [u8]>
{
    type Target = [u8];
    
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("the frame is skipped")]
    Skipped,
    #[error(transparent)]
    Error(io::Error),
}

impl From<io::Error> for FrameError {
    fn from(value: io::Error) -> Self {
        match value.kind() {
            ErrorKind::WouldBlock => Self::Skipped,
            _ => Self::Error(value),
        }
    }
}
