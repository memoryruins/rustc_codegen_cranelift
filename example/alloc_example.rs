#![feature(start, box_syntax, core_intrinsics, alloc, alloc_error_handler, libstd_sys_internals, std_internals, panic_internals)]
#![feature(unwind_attributes)]
//#![no_std]

extern crate core;

use std::io::{self, Error, ErrorKind, Write};
use std::fmt;

#[link(name = "c")]
extern "C" {
    fn puts(s: *const u8);
}

fn main() {

    let stderr = ::std::io::stderr();
    let mut stderr = stderr.lock();
    let mut output = Adaptor { inner: &mut stderr, error: Ok(()) };
    match fmt::write(&mut output, format_args!("athread '{}' panicked at ...", "<unknown>")) {
        Ok(()) => {},
        Err(..) => {
            unsafe { std::intrinsics::abort(); }
        }
    };
}

struct Adaptor<'a, T: ?Sized + 'a> {
    inner: &'a mut T,
    error: io::Result<()>,
}

impl<'a, T: io::Write + ?Sized> fmt::Write for Adaptor<'a, T> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        unsafe { puts("before\0" as *const str as *const u8); }
        match self.inner.write_all(s.as_bytes()) {
            Ok(()) => {
                unsafe { puts("after\0" as *const str as *const u8); }
                Ok(())
            }
            Err(e) => {
                unsafe { std::intrinsics::abort(); }
            }
        }
    }
}
