use crate::buf::{IoBuf, IoBufMut};

use std::ops;

pub struct SliceMut<T> {
    buf: T,
    begin: usize,
    end: usize,
}

impl<T: IoBufMut> SliceMut<T> {
    /// Create a SliceMut from a buffer and range.
    pub fn new(buf: T, begin: usize, end: usize) -> SliceMut<T> {
        assert!(end <= buf.bytes_total());
        assert!(begin <= buf.bytes_init());
        assert!(begin <= end);
        SliceMut { buf, begin, end }
    }
}

impl<T> SliceMut<T> {
    /// Create a SliceMut from a buffer and range without boundary checking
    ///
    /// # Safety
    /// begin must be initialized, and the end must be within the buffer capacity
    pub unsafe fn new_unchecked(buf: T, begin: usize, end: usize) -> SliceMut<T> {
        SliceMut { buf, begin, end }
    }

    /// Offset in the underlying buffer at which this slice starts.
    pub fn begin(&self) -> usize {
        self.begin
    }

    /// Offset in the underlying buffer at which this slice ends.
    pub fn end(&self) -> usize {
        self.end
    }

    /// Gets a reference to the underlying buffer.
    ///
    /// This method escapes the slice's view.
    pub fn get_ref(&self) -> &T {
        &self.buf
    }

    /// Gets a mutable reference to the underlying buffer.
    ///
    /// This method escapes the slice's view
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.buf
    }

    /// Unwraps this `Slice`, returning the underyling buffer
    pub fn into_inner(self) -> T {
        self.buf
    }
}

impl<T: IoBuf> ops::Deref for SliceMut<T> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        let buf_bytes = super::deref(&self.buf);
        let end = std::cmp::min(self.end, buf_bytes.len());
        &buf_bytes[self.begin..end]
    }
}

impl<T: IoBufMut> ops::DerefMut for SliceMut<T> {
    fn deref_mut(&mut self) -> &mut [u8] {
        let buf_bytes = super::deref_mut(&mut self.buf);
        let end = std::cmp::min(self.end, buf_bytes.len());
        &mut buf_bytes[self.begin..end]
    }
}

unsafe impl<T: IoBuf> IoBuf for SliceMut<T> {
    fn read_ptr(&self) -> *const u8 {
        super::deref(&self.buf)[self.begin..].as_ptr()
    }

    fn bytes_init(&self) -> usize {
        ops::Deref::deref(self).len()
    }

    fn bytes_total(&self) -> usize {
        self.end - self.begin
    }
}

unsafe impl<T: IoBufMut> IoBufMut for SliceMut<T> {
    fn write_ptr(&mut self) -> *mut u8 {
        let buf_bytes = super::deref_mut(&mut self.buf);
        buf_bytes[self.begin..].as_mut_ptr()
    }

    unsafe fn set_init(&mut self, pos: usize) {
        self.buf.set_init(self.begin + pos);
    }
}
