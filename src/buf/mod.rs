mod io_buf;
mod io_buf_mut;
mod slice;
mod slice_mut;

pub use io_buf::IoBuf;
pub use io_buf_mut::IoBufMut;
pub use slice::Slice;
pub use slice_mut::SliceMut;

pub(crate) fn deref(buf: &impl IoBuf) -> &[u8] {
    // Safety: The `IoBuf` trait is marked as unsafe and is expected to be
    // implemented correctly
    unsafe { std::slice::from_raw_parts(buf.read_ptr(), buf.bytes_init()) }
}

pub(crate) fn deref_mut(buf: &mut impl IoBufMut) -> &mut [u8] {
    // Safety: The `IoBufMut` trait is marked as unsafe and is expected to be
    // implemented correctly
    unsafe { std::slice::from_raw_parts_mut(buf.write_ptr(), buf.bytes_init()) }
}
