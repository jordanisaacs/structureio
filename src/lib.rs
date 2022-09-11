#![feature(ptr_const_cast)]
#![feature(io_error_more)]

extern crate alloc;

#[macro_use(defer)]
extern crate scopeguard;

pub mod executor;
pub mod macros;
pub mod scheduler;
pub mod task;
pub mod utils;

pub mod buf;
pub mod driver;
