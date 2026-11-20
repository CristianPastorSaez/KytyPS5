//! Validation of the dynamic section (`PT_DYNAMIC`) and the tables it points to, before the
//! C++ runtime linker turns tag values into pointers, indices and relocation targets.
//!
//! Errors are memory-safety violations (something would be read or written out of bounds).
//! Warnings are inconsistencies that stay in bounds (a name running past `STRSZ` but terminated
//! inside the data buffer); the caller logs them and carries on.

/// Tag values, as in `src/loader/elf.h`.
pub mod tag {
	pub const NULL: i64 = 0;
	pub const PLTRELSZ: i64 = 0x02;
	pub const HASH: i64 = 0x04;
	pub const STRTAB: i64 = 0x05;
	pub const SYMTAB: i64 = 0x06;
	pub const RELA: i64 = 0x07;
	pub const RELASZ: i64 = 0x08;
	pub const RELAENT: i64 = 0x09;
	pub const STRSZ: i64 = 0x0a;
	pub const SYMENT: i64 = 0x0b;
	pub const SONAME: i64 = 0x0e;
	pub const JMPREL: i64 = 0x17;
	pub const OS_MODULE_INFO: i64 = 0x6100_000d;
	pub const OS_NEEDED_MODULE: i64 = 0x6100_000f;
	pub const OS_EXPORT_LIB: i64 = 0x6100_0013;
	pub const OS_IMPORT_LIB: i64 = 0x6100_0015;
	pub const OS_HASH: i64 = 0x6100_0025;
	pub const OS_JMPREL: i64 = 0x6100_0029;
	pub const OS_PLTRELSZ: i64 = 0x6100_002d;
	pub const OS_RELA: i64 = 0x6100_002f;
	pub const OS_RELASZ: i64 = 0x6100_0031;
	pub const OS_RELAENT: i64 = 0x6100_0033;
	pub const OS_STRTAB: i64 = 0x6100_0035;
	pub const OS_STRSZ: i64 = 0x6100_0037;
	pub const OS_SYMTAB: i64 = 0x6100_0039;
	pub const OS_SYMENT: i64 = 0x6100_003b;
	pub const OS_HASHSZ: i64 = 0x6100_003d;
	pub const OS_SYMTABSZ: i64 = 0x6100_003f;
	pub const OS_MODULE_INFO_1: i64 = 0x6100_0043;
	pub const OS_NEEDED_MODULE_1: i64 = 0x6100_0045;
	pub const OS_EXPORT_LIB_1: i64 = 0x6100_0047;
	pub const OS_IMPORT_LIB_1: i64 = 0x6100_0049;
}

pub const DYN_SIZE: u64 = 16;
pub const SYM_SIZE: u64 = 24;
pub const RELA_SIZE: u64 = 24;

const R_X86_64_64: u32 = 1;
const R_X86_64_GLOB_DAT: u32 = 6;
const R_X86_64_JUMP_SLOT: u32 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Code {
	/// No `DT_NULL` inside `PT_DYNAMIC`: tag lookups would run off the buffer.
	NotTerminated = 20,
	/// A `DT_OS_*` table does not fit the dynamic data segment.
	TableOutOfData = 21,
	/// A `DT_*` table does not fit the loaded image.
	TableOutOfImage = 22,
	/// Table size not a multiple of its entry size, or entry size not the expected one.
	BadEntrySize = 23,
	/// Relocation target `r_offset .. r_offset + 8` outside the module image.
	RelocOutOfImage = 24,
	/// Relocation references a symbol index past the symbol table.
	SymbolIndexOutOfRange = 25,
	/// Name offset (symbol, SONAME, module or library) at or past `STRSZ`.
	NameOutOfRange = 26,
	/// Name has no NUL before the end of the dynamic data segment.
	NameNotTerminated = 27,
	/// Warning: name terminates past `STRSZ` (still inside the dynamic data segment).
	NamePastStrsz = 40,
}

impl Code {
	pub const ALL: [Code; 9] = [
		Code::NotTerminated,
		Code::TableOutOfData,
		Code::TableOutOfImage,
		Code::BadEntrySize,
		Code::RelocOutOfImage,
		Code::SymbolIndexOutOfRange,
		Code::NameOutOfRange,
		Code::NameNotTerminated,
		Code::NamePastStrsz,
	];

	pub fn name(self) -> &'static std::ffi::CStr {
		match self {
			Code::NotTerminated => c"dynamic section has no DT_NULL terminator",
			Code::TableOutOfData => c"DT_OS_* table out of dynamic data bounds",
			Code::TableOutOfImage => c"DT_* table out of module image bounds",
			Code::BadEntrySize => c"dynamic table size or entry size mismatch",
			Code::RelocOutOfImage => c"relocation target outside module image",
			Code::SymbolIndexOutOfRange => c"relocation symbol index out of range",
			Code::NameOutOfRange => c"name offset past string table size",
			Code::NameNotTerminated => c"name not NUL-terminated inside dynamic data",
			Code::NamePastStrsz => c"name terminates past string table size",
		}
	}
}

/// A problem and the index of the entry that caused it (dynamic entry, symbol or relocation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finding {
	pub code: Code,
	pub index: u64,
}

#[derive(Debug, Default)]
pub struct Report {
	pub warnings: Vec<Finding>,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
	u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or_default())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
	u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default())
}

/// Decoded `(tag, value)` entries up to (excluding) `DT_NULL`.
fn entries(dynamic: &[u8]) -> Result<Vec<(i64, u64)>, Finding> {
	let mut out = Vec::new();
	for e in dynamic.as_chunks::<{ DYN_SIZE as usize }>().0.iter() {
		let t = u64_at(e, 0) as i64;
		if t == tag::NULL {
			return Ok(out);
		}
		out.push((t, u64_at(e, 8)));
	}
	Err(Finding { code: Code::NotTerminated, index: out.len() as u64 })
}

/// First value for `t`, as `Elf64::GetDynValue` returns.
fn first(entries: &[(i64, u64)], t: i64) -> Option<(u64, u64)> {
	entries.iter().position(|e| e.0 == t).map(|i| (entries[i].1, i as u64))
}

/// Where a table lives once validated.
enum Place {
	Absent,
	/// Byte range inside the dynamic data segment (contents can be checked).
	Data(std::ops::Range<usize>),
	/// Inside the loaded image (only the range can be checked here).
	Image,
}

/// Resolves the `DT_OS_*` (dynamic data) or `DT_*` (image) variant of a table and checks its range.
fn table(e: &[(i64, u64)], os: i64, std_tag: i64, size: u64, data: &[u8], image_size: u64) -> Result<Place, Finding> {
	if let Some((off, i)) = first(e, os) {
		let bad = Finding { code: Code::TableOutOfData, index: i };
		let end = off.checked_add(size).ok_or(bad)?;
		if end > data.len() as u64 {
			return Err(bad);
		}
		return Ok(Place::Data(off as usize..end as usize));
	}
	if let Some((ptr, i)) = first(e, std_tag) {
		let end = ptr.checked_add(size).ok_or(Finding { code: Code::TableOutOfImage, index: i })?;
		if end > image_size || ptr >= image_size {
			return Err(Finding { code: Code::TableOutOfImage, index: i });
		}
		return Ok(Place::Image);
	}
	Ok(Place::Absent)
}

fn value(e: &[(i64, u64)], tags: &[i64]) -> u64 {
	tags.iter().find_map(|t| first(e, *t)).map_or(0, |v| v.0)
}

/// Checks a name at `off` inside the string table `strtab` (a range of `data`).
fn check_name(data: &[u8], strtab: &std::ops::Range<usize>, off: u64, index: u64, report: &mut Report) -> Result<(), Finding> {
	let strsz = (strtab.end - strtab.start) as u64;
	if off >= strsz {
		return Err(Finding { code: Code::NameOutOfRange, index });
	}
	let start = strtab.start + off as usize;
	match data[start..].iter().position(|b| *b == 0) {
		None => Err(Finding { code: Code::NameNotTerminated, index }),
		Some(len) if start + len >= strtab.end => {
			report.warnings.push(Finding { code: Code::NamePastStrsz, index });
			Ok(())
		}
		Some(_) => Ok(()),
	}
}

/// Validates `PT_DYNAMIC` (`dynamic`), the `PT_OS_DYNLIBDATA` segment (`data`) and every table,
/// relocation target and name the runtime linker will use for a module of `image_size` bytes.
pub fn validate(dynamic: &[u8], data: &[u8], image_size: u64) -> Result<Report, Finding> {
	let e = entries(dynamic)?;
	let mut report = Report::default();

	for (sz, ent, entry) in [
		(&[tag::OS_SYMTABSZ][..], &[tag::OS_SYMENT, tag::SYMENT][..], SYM_SIZE),
		(&[tag::OS_RELASZ, tag::RELASZ][..], &[tag::OS_RELAENT, tag::RELAENT][..], RELA_SIZE),
		(&[tag::OS_PLTRELSZ, tag::PLTRELSZ][..], &[][..], RELA_SIZE),
	] {
		let size = value(&e, sz);
		let ent_size = value(&e, ent);
		if !size.is_multiple_of(entry) || (ent_size != 0 && ent_size != entry) {
			return Err(Finding { code: Code::BadEntrySize, index: 0 });
		}
	}

	let strsz = value(&e, &[tag::OS_STRSZ, tag::STRSZ]);
	let symtabsz = value(&e, &[tag::OS_SYMTABSZ]);
	let relasz = value(&e, &[tag::OS_RELASZ, tag::RELASZ]);
	let pltrelsz = value(&e, &[tag::OS_PLTRELSZ, tag::PLTRELSZ]);
	let hashsz = value(&e, &[tag::OS_HASHSZ]);

	let strtab = table(&e, tag::OS_STRTAB, tag::STRTAB, strsz, data, image_size)?;
	let symtab = table(&e, tag::OS_SYMTAB, tag::SYMTAB, symtabsz, data, image_size)?;
	let rela = table(&e, tag::OS_RELA, tag::RELA, relasz, data, image_size)?;
	let jmprel = table(&e, tag::OS_JMPREL, tag::JMPREL, pltrelsz, data, image_size)?;
	table(&e, tag::OS_HASH, tag::HASH, hashsz, data, image_size)?;

	// Symbol indices can only be checked against a table whose size we know (dynamic data).
	let sym_count = matches!(symtab, Place::Data(_)).then_some(symtabsz / SYM_SIZE);
	for place in [&rela, &jmprel] {
		let Place::Data(r) = place else { continue };
		for (i, rec) in data[r.clone()].as_chunks::<{ RELA_SIZE as usize }>().0.iter().enumerate() {
			let index = i as u64;
			let target_end = u64_at(rec, 0).checked_add(8);
			if target_end.is_none_or(|end| end > image_size) {
				return Err(Finding { code: Code::RelocOutOfImage, index });
			}
			let r_type = u32_at(rec, 8);
			let sym = u64::from(u32_at(rec, 12));
			let uses_symbol = matches!(r_type, R_X86_64_64 | R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT);
			if uses_symbol && sym_count.is_some_and(|count| sym >= count) {
				return Err(Finding { code: Code::SymbolIndexOutOfRange, index });
			}
		}
	}

	if let Place::Data(s) = &strtab {
		if let Place::Data(syms) = &symtab {
			for (i, sym) in data[syms.clone()].as_chunks::<{ SYM_SIZE as usize }>().0.iter().enumerate() {
				check_name(data, s, u64::from(u32_at(sym, 0)), i as u64, &mut report)?;
			}
		}
		for (i, (t, v)) in e.iter().enumerate() {
			let off = match *t {
				tag::SONAME => *v,
				tag::OS_NEEDED_MODULE
				| tag::OS_NEEDED_MODULE_1
				| tag::OS_MODULE_INFO
				| tag::OS_MODULE_INFO_1
				| tag::OS_IMPORT_LIB
				| tag::OS_IMPORT_LIB_1
				| tag::OS_EXPORT_LIB
				| tag::OS_EXPORT_LIB_1 => *v & 0xffff_ffff,
				_ => continue,
			};
			check_name(data, s, off, i as u64, &mut report)?;
		}
	}
	Ok(report)
}
