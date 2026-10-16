use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout};
use std::ptr::NonNull;

pub const DIRECT_ALIGN: usize = 4096;

/// Buffer heap-allocated with explicit page/sector alignment (4096 bytes)
/// suitable for Linux `O_DIRECT` reads and writes on 512e and 4Kn block devices.
pub struct AlignedBuf {
    ptr: NonNull<u8>,
    len: usize,
    layout: Layout,
}

unsafe impl Send for AlignedBuf {}
unsafe impl Sync for AlignedBuf {}

impl AlignedBuf {
    pub fn new(len: usize, align: usize) -> Self {
        assert!(align.is_power_of_two(), "alignment must be a power of two");
        let min_len = len.max(align);
        let alloc_len = (min_len + align - 1) & !(align - 1);
        let layout = Layout::from_size_align(alloc_len, align).expect("valid aligned layout");
        let raw = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).unwrap_or_else(|| handle_alloc_error(layout));
        Self {
            ptr,
            len: alloc_len,
            layout,
        }
    }

    pub fn new_4k(len: usize) -> Self {
        Self::new(len, DIRECT_ALIGN)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn alignment(&self) -> usize {
        self.layout.align()
    }

    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_buf_is_4096_byte_aligned() {
        let mut buf = AlignedBuf::new_4k(4 * 1024 * 1024);
        assert_eq!((buf.as_ptr() as usize) % DIRECT_ALIGN, 0);
        assert_eq!(buf.len(), 4 * 1024 * 1024);
        assert_eq!(buf.alignment(), DIRECT_ALIGN);

        let last = buf.len() - 1;
        buf.as_mut_slice()[0] = 0xAA;
        buf.as_mut_slice()[last] = 0x55;
        assert_eq!(buf.as_slice()[0], 0xAA);
        assert_eq!(buf.as_slice()[last], 0x55);
    }

    #[test]
    fn aligned_buf_rounds_up_unaligned_size() {
        let buf = AlignedBuf::new_4k(5000);
        assert_eq!((buf.as_ptr() as usize) % DIRECT_ALIGN, 0);
        assert_eq!(buf.len(), 8192);
    }
}
