use crate::buf::Slice;

use std::ops;

/// An `io-uring` compatible buffer.
///
/// The `IoBuf` trait is implemented by buffer types that can be passed to
/// io-uring operations. Users will not need to use this trait directly, except
/// for the [`slice`] method.
///
/// # Slicing
///
/// Because buffers are passed by ownership to the runtime, Rust's slice API
/// (`&buf[..]`) cannot be used. Instead, `tokio-uring` provides an owned slice
/// API: [`slice()`]. The method takes ownership fo the buffer and returns a
/// `Slice<Self>` type that tracks the requested offset.
///
/// # Implementation notes
///
/// Buffers passed to `io-uring` operations must reference a stable memory
/// region. While the runtime holds ownership to a buffer, the pointer returned
/// by `stable_ptr` must remain valid even if the `IoBuf` value is moved.
///
/// [`slice()`]: IoBuf::slice
pub unsafe trait IoBuf: Unpin + 'static {
    /// Returns a raw pointer to the vector’s buffer.
    ///
    /// This method is to be used by the `tokio-uring` runtime and it is not
    /// expected for users to call it directly.
    ///
    /// The implementation must ensure that, while the `tokio-uring` runtime
    /// owns the value, the pointer returned by `stable_ptr` **does not**
    /// change.
    fn read_ptr(&self) -> *const u8;

    /// Number of initialized bytes.
    ///
    /// This method is to be used by the `tokio-uring` runtime and it is not
    /// expected for users to call it directly.
    ///
    /// For `Vec`, this is identical to `len()`.
    fn bytes_init(&self) -> usize;

    /// Total size of the buffer, including uninitialized memory, if any.
    ///
    /// This method is to be used by the `tokio-uring` runtime and it is not
    /// expected for users to call it directly.
    ///
    /// For `Vec`, this is identical to `capacity()`.
    fn bytes_total(&self) -> usize;

    /// Returns a view of the buffer with the specified range.
    ///
    /// This method is similar to Rust's slicing (`&buf[..]`), but takes
    /// ownership of the buffer.
    ///
    /// # Examples
    ///
    fn slice(self, range: impl ops::RangeBounds<usize>) -> Slice<Self>
    where
        Self: Sized,
    {
        let (begin, end) = parse_range(range, self.bytes_init());
        Slice::new(self, begin, end)
    }

    unsafe fn slice_unchecked(self, range: impl ops::RangeBounds<usize>) -> Slice<Self>
    where
        Self: Sized,
    {
        let (begin, end) = parse_range(range, self.bytes_init());
        Slice::new_unchecked(self, begin, end)
    }
}

unsafe impl IoBuf for Vec<u8> {
    fn read_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn bytes_init(&self) -> usize {
        self.len()
    }

    fn bytes_total(&self) -> usize {
        self.capacity()
    }
}

unsafe impl IoBuf for &'static [u8] {
    fn read_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn bytes_init(&self) -> usize {
        <[u8]>::len(self)
    }

    fn bytes_total(&self) -> usize {
        self.bytes_init()
    }
}

unsafe impl IoBuf for &'static str {
    fn read_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn bytes_init(&self) -> usize {
        <str>::len(self)
    }

    fn bytes_total(&self) -> usize {
        self.bytes_init()
    }
}

fn parse_range(range: impl ops::RangeBounds<usize>, size: usize) -> (usize, usize) {
    use core::ops::Bound;

    let begin = match range.start_bound() {
        Bound::Included(&n) => n,
        Bound::Excluded(&n) => n + 1,
        Bound::Unbounded => 0,
    };

    let end = match range.end_bound() {
        Bound::Included(&n) => n.checked_add(1).expect("out of range"),
        Bound::Excluded(&n) => n,
        Bound::Unbounded => size,
    };

    (begin, end)
}
