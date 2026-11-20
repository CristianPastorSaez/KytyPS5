//! Bounds-checked reader for the SELF/ELF header tables used by the KytyPS5 loader.
//!
//! The C++ loader used to `Seek` + `Read` header tables at offsets and counts taken straight
//! from the file. This crate does the same reads through a caller-supplied callback, but checks
//! every range against the file size with overflow-safe arithmetic first, and returns the raw
//! table bytes. Semantic checks (magic, machine, ABI) stay in C++ (`Elf64::IsValid`).
//!
//! The core ([`parse`]) is safe Rust; `unsafe` is confined to the [`ffi`] module.

pub mod ffi;

/// On-disk sizes. The C++ side `static_assert`s its structs against these.
pub const SELF_HEADER_SIZE: u64 = 32;
pub const SELF_SEGMENT_SIZE: u64 = 32;
pub const EHDR_SIZE: u64 = 64;
pub const PHDR_SIZE: u64 = 56;
pub const SHDR_SIZE: u64 = 64;

/// Largest single table we will allocate for. The biggest legitimate one is a string table;
/// the C++ side stores its size as `uint32_t`.
const MAX_TABLE_BYTES: u64 = u32::MAX as u64;

/// Random-access byte source (a file, in production).
pub trait Source {
	/// Total size in bytes.
	fn size(&self) -> u64;
	/// Fill `buf` from `offset`. Returns false on a short or failed read.
	fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> bool;
}

impl Source for &[u8] {
	fn size(&self) -> u64 {
		self.len() as u64
	}
	fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> bool {
		let Ok(start) = usize::try_from(offset) else { return false };
		match start.checked_add(buf.len()).and_then(|end| self.get(start..end)) {
			Some(src) => {
				buf.copy_from_slice(src);
				true
			}
			None => false,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Error {
	/// Smaller than an ELF header.
	TooSmall = 1,
	SelfSegmentsOutOfRange = 2,
	EhdrOutOfRange = 3,
	PhdrsOutOfRange = 4,
	ShdrsOutOfRange = 5,
	StrtabOutOfRange = 6,
	/// Non-empty string table whose last byte is not NUL: name lookups could run off the end.
	StrtabNotTerminated = 7,
	ReadFailed = 8,
}

impl Error {
	pub const ALL: [Error; 8] = [
		Error::TooSmall,
		Error::SelfSegmentsOutOfRange,
		Error::EhdrOutOfRange,
		Error::PhdrsOutOfRange,
		Error::ShdrsOutOfRange,
		Error::StrtabOutOfRange,
		Error::StrtabNotTerminated,
		Error::ReadFailed,
	];

	pub fn name(self) -> &'static std::ffi::CStr {
		match self {
			Error::TooSmall => c"file smaller than an ELF header",
			Error::SelfSegmentsOutOfRange => c"SELF segment table out of file bounds",
			Error::EhdrOutOfRange => c"ELF header out of file bounds",
			Error::PhdrsOutOfRange => c"program header table out of file bounds",
			Error::ShdrsOutOfRange => c"section header table out of file bounds",
			Error::StrtabOutOfRange => c"section name string table out of file bounds",
			Error::StrtabNotTerminated => c"section name string table not NUL-terminated",
			Error::ReadFailed => c"read failed",
		}
	}
}

/// Header tables read from one file. Every table lies entirely within the file.
#[derive(Debug, Default)]
pub struct Layout {
	/// First 32 bytes when the file is at least that big (C++ derives `IsSelf` from them).
	pub self_header: Option<Vec<u8>>,
	/// True when `self_header` carries a recognised SELF magic and ident tail.
	pub is_self: bool,
	pub self_segments: Vec<u8>,
	/// File offset of the ELF header (after the SELF header and segment table, or 0).
	pub ehdr_offset: u64,
	pub ehdr: Vec<u8>,
	/// Empty when the ELF magic is wrong: C++ `IsValid` rejects the file and logs why.
	pub phdrs: Vec<u8>,
	/// Only read for plain ELF files, as before (SELF files skip the section table).
	pub shdrs: Vec<u8>,
	pub strtab: Option<Vec<u8>>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
	u16::from_le_bytes([b[at], b[at + 1]])
}
fn u64_at(b: &[u8], at: usize) -> u64 {
	u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default())
}

/// Same rule as C++ `Elf64::IsSelf`.
fn is_known_self(h: &[u8]) -> bool {
	let magic = h[0..4] == [0x4f, 0x15, 0x3d, 0x1d] || h[0..4] == [0x54, 0x14, 0xf5, 0xee];
	let unknown = u16_at(h, 26);
	let tail = (h[4..12] == [0x00, 0x01, 0x01, 0x12, 0x01, 0x01, 0x00, 0x00] && unknown == 0x22)
		|| (h[4..12] == [0x10, 0x01, 0x01, 0x12, 0x01, 0x01, 0x00, 0x10] && unknown == 0x32);
	magic && tail
}

/// Reads `count * entry` bytes at `offset`, or `err` if that range is not inside the file.
fn read_table<S: Source>(src: &mut S, offset: u64, count: u64, entry: u64, err: Error) -> Result<Vec<u8>, Error> {
	let len = count.checked_mul(entry).ok_or(err)?;
	let end = offset.checked_add(len).ok_or(err)?;
	if end > src.size() || len > MAX_TABLE_BYTES {
		return Err(err);
	}
	let len = usize::try_from(len).map_err(|_| err)?;
	let mut buf = vec![0u8; len];
	if !src.read_at(offset, &mut buf) {
		return Err(Error::ReadFailed);
	}
	Ok(buf)
}

/// Reads the SELF header, SELF segment table, ELF header, program headers and (plain ELF only)
/// section headers plus section-name string table, mirroring `Elf64::Open`.
pub fn parse<S: Source>(src: &mut S) -> Result<Layout, Error> {
	let mut out = Layout::default();

	if src.size() >= SELF_HEADER_SIZE {
		let header = read_table(src, 0, 1, SELF_HEADER_SIZE, Error::ReadFailed)?;
		out.is_self = is_known_self(&header);
		if out.is_self {
			let segments = u64::from(u16_at(&header, 24));
			out.self_segments =
				read_table(src, SELF_HEADER_SIZE, segments, SELF_SEGMENT_SIZE, Error::SelfSegmentsOutOfRange)?;
			out.ehdr_offset = SELF_HEADER_SIZE + segments * SELF_SEGMENT_SIZE;
		}
		out.self_header = Some(header);
	}

	if src.size() < EHDR_SIZE {
		return Err(Error::TooSmall);
	}
	out.ehdr = read_table(src, out.ehdr_offset, 1, EHDR_SIZE, Error::EhdrOutOfRange)?;
	if out.ehdr[0..4] != *b"\x7fELF" {
		return Ok(out);
	}

	let e = &out.ehdr;
	let (phoff, shoff) = (u64_at(e, 32), u64_at(e, 40));
	let (phnum, shnum, shstrndx) = (u64::from(u16_at(e, 56)), u64::from(u16_at(e, 60)), u64::from(u16_at(e, 62)));

	let phdr_at = out.ehdr_offset.checked_add(phoff).ok_or(Error::PhdrsOutOfRange)?;
	out.phdrs = read_table(src, phdr_at, phnum, PHDR_SIZE, Error::PhdrsOutOfRange)?;

	if !out.is_self && shnum != 0 {
		let shdr_at = out.ehdr_offset.checked_add(shoff).ok_or(Error::ShdrsOutOfRange)?;
		out.shdrs = read_table(src, shdr_at, shnum, SHDR_SIZE, Error::ShdrsOutOfRange)?;
		if shstrndx < shnum {
			let at = usize::try_from(shstrndx * SHDR_SIZE).map_err(|_| Error::StrtabOutOfRange)?;
			let (offset, size) = (u64_at(&out.shdrs, at + 24), u64_at(&out.shdrs, at + 32));
			if size != 0 {
				let table = read_table(src, offset, size, 1, Error::StrtabOutOfRange)?;
				if table.last() != Some(&0) {
					return Err(Error::StrtabNotTerminated);
				}
				out.strtab = Some(table);
			}
		}
	}
	Ok(out)
}

#[cfg(test)]
mod tests;
