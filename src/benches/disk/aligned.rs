//! A heap buffer aligned to 4096 bytes, as required by `O_DIRECT` /
//! `FILE_FLAG_NO_BUFFERING` (buffer address, length and file offset must all be
//! sector multiples; 4096 satisfies every common sector size).

use std::alloc::{Layout, alloc_zeroed, dealloc, handle_alloc_error};
use std::slice;

const ALIGN: usize = 4096;

pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
}

impl AlignedBuf {
    /// `len` must be non-zero and a multiple of 4096.
    pub fn new(len: usize) -> Self {
        assert!(
            len > 0 && len.is_multiple_of(ALIGN),
            "AlignedBuf length must be a positive multiple of {ALIGN}"
        );
        let layout = Layout::from_size_align(len, ALIGN).expect("valid layout");
        // SAFETY: layout has non-zero size (len > 0).
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            handle_alloc_error(layout);
        }
        Self { ptr, len }
    }

    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: ptr is a valid allocation of `len` initialised bytes, and the
        // borrow of `self` bounds the returned slice's lifetime.
        unsafe { slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as above; `&mut self` guarantees exclusive access.
        unsafe { slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.len, ALIGN).expect("valid layout");
        // SAFETY: ptr came from `alloc_zeroed` with this exact layout.
        unsafe { dealloc(self.ptr, layout) }
    }
}

// SAFETY: `AlignedBuf` uniquely owns its allocation; there is no shared interior
// state, so it is safe to move between threads.
unsafe impl Send for AlignedBuf {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_is_aligned_and_zeroed() {
        let buf = AlignedBuf::new(8192);
        assert_eq!(buf.as_slice().as_ptr() as usize % ALIGN, 0);
        assert!(buf.as_slice().iter().all(|&b| b == 0));
    }

    #[test]
    fn round_trips_through_mut_slice() {
        let mut buf = AlignedBuf::new(4096);
        buf.as_mut_slice()[0] = 0xAB;
        buf.as_mut_slice()[4095] = 0xCD;
        assert_eq!(buf.as_slice()[0], 0xAB);
        assert_eq!(buf.as_slice()[4095], 0xCD);
    }
}
