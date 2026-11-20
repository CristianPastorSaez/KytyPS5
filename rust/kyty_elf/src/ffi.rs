//! C ABI. The only `unsafe` in the crate lives here: calling the C++ read callback and
//! dereferencing the pointers C++ hands us. See `include/kyty_elf.h` for the contract.

use crate::{parse, Error, Layout, Source};
use std::ffi::{c_char, c_void};

/// Reads `size` bytes at `offset` into `dst`; returns non-zero only if all of them were read.
pub type ReadFn = unsafe extern "C" fn(ctx: *mut c_void, offset: u64, dst: *mut u8, size: u64) -> i32;

struct Callback {
	ctx: *mut c_void,
	read: ReadFn,
	size: u64,
}

impl Source for Callback {
	fn size(&self) -> u64 {
		self.size
	}
	fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> bool {
		// SAFETY: `buf` is a live, exclusively borrowed buffer of exactly `buf.len()` bytes, and
		// the caller of `kyty_elf_open` guarantees `read`/`ctx` are valid for the call.
		unsafe { (self.read)(self.ctx, offset, buf.as_mut_ptr(), buf.len() as u64) != 0 }
	}
}

/// Opaque handle owning the validated tables.
pub struct KytyElf(Layout);

/// Parses the headers of a file of `file_size` bytes read through `read(ctx, ...)`.
/// Returns 0 and stores a handle in `*out` (free it with `kyty_elf_free`), or an error code
/// (see `kyty_elf_error_name`) and stores null.
///
/// # Safety
/// `out` must be valid for writes; `read` must be a valid function for the duration of the
/// call and honour its contract for `ctx`.
#[no_mangle]
pub unsafe extern "C" fn kyty_elf_open(ctx: *mut c_void, read: Option<ReadFn>, file_size: u64, out: *mut *mut KytyElf) -> i32 {
	if out.is_null() {
		return Error::ReadFailed as i32;
	}
	*out = std::ptr::null_mut();
	let Some(read) = read else { return Error::ReadFailed as i32 };
	match parse(&mut Callback { ctx, read, size: file_size }) {
		Ok(layout) => {
			*out = Box::into_raw(Box::new(KytyElf(layout)));
			0
		}
		Err(e) => e as i32,
	}
}

/// # Safety
/// `elf` must be null or a handle from `kyty_elf_open` not yet freed.
#[no_mangle]
pub unsafe extern "C" fn kyty_elf_free(elf: *mut KytyElf) {
	if !elf.is_null() {
		drop(Box::from_raw(elf));
	}
}

/// Returns a pointer to the raw bytes of `table` (`KYTY_ELF_TABLE_*` in the header) and stores its length in `*size`; null and 0
/// when the table is absent. The bytes live until `kyty_elf_free`.
///
/// # Safety
/// `elf` must be a live handle; `size` must be valid for writes.
#[no_mangle]
pub unsafe extern "C" fn kyty_elf_table(elf: *const KytyElf, table: i32, size: *mut u64) -> *const u8 {
	let bytes: &[u8] = match (elf.as_ref(), table) {
		(Some(KytyElf(l)), 0) => l.self_header.as_deref().unwrap_or(&[]),
		(Some(KytyElf(l)), 1) => &l.self_segments,
		(Some(KytyElf(l)), 2) => &l.ehdr,
		(Some(KytyElf(l)), 3) => &l.phdrs,
		(Some(KytyElf(l)), 4) => &l.shdrs,
		(Some(KytyElf(l)), 5) => l.strtab.as_deref().unwrap_or(&[]),
		_ => &[],
	};
	if !size.is_null() {
		*size = bytes.len() as u64;
	}
	if bytes.is_empty() {
		std::ptr::null()
	} else {
		bytes.as_ptr()
	}
}

/// 1 if the file has a recognised SELF header, else 0.
///
/// # Safety
/// `elf` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn kyty_elf_is_self(elf: *const KytyElf) -> i32 {
	i32::from(elf.as_ref().is_some_and(|KytyElf(l)| l.is_self))
}

/// File offset of the ELF header.
///
/// # Safety
/// `elf` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn kyty_elf_ehdr_offset(elf: *const KytyElf) -> u64 {
	elf.as_ref().map_or(0, |KytyElf(l)| l.ehdr_offset)
}

/// Static, NUL-terminated description of an error code from `kyty_elf_open`.
#[no_mangle]
pub extern "C" fn kyty_elf_error_name(code: i32) -> *const c_char {
	match code {
		0 => c"ok".as_ptr(),
		_ => Error::ALL.iter().find(|e| **e as i32 == code).map_or(c"unknown error", |e| e.name()).as_ptr(),
	}
}
