//! Tests for segment checks (audit A6–A8) and dynamic validation (A1–A5).

use crate::dynamic::{tag, validate, Code, Finding, RELA_SIZE, SYM_SIZE};
use crate::tests::{self_wrapped, synthetic_elf};
use crate::{parse, Error, EHDR_SIZE};

fn put(f: &mut [u8], at: usize, v: u64, n: usize) {
	f[at..at + n].copy_from_slice(&v.to_le_bytes()[..n]);
}

// ---- segments (A6–A8) -----------------------------------------------------------------------

/// Turns program header 0 of the synthetic ELF into a PT_LOAD.
fn with_load(filesz: u64, memsz: u64, align: u64, vaddr: u64, offset: u64) -> Vec<u8> {
	let mut f = synthetic_elf();
	let p = EHDR_SIZE as usize;
	put(&mut f, p, 1, 4); // PT_LOAD
	put(&mut f, p + 8, offset, 8);
	put(&mut f, p + 16, vaddr, 8);
	put(&mut f, p + 32, filesz, 8);
	put(&mut f, p + 40, memsz, 8);
	put(&mut f, p + 48, align, 8);
	f
}

#[test]
fn accepts_sane_load_segment() {
	assert!(parse(&mut &with_load(16, 0x4000, 0x4000, 0, 0)[..]).is_ok());
}

#[test]
fn rejects_filesz_over_memsz() {
	assert_eq!(parse(&mut &with_load(64, 32, 0, 0, 0)[..]).unwrap_err(), Error::SegmentFileExceedsMemory);
}

#[test]
fn rejects_non_power_of_two_alignment() {
	assert_eq!(parse(&mut &with_load(16, 32, 0x3000, 0, 0)[..]).unwrap_err(), Error::SegmentBadAlignment);
}

#[test]
fn rejects_vaddr_overflow() {
	assert_eq!(parse(&mut &with_load(16, 0x4000, 0x4000, u64::MAX - 0x100, 0)[..]).unwrap_err(), Error::SegmentAddressOverflow);
}

#[test]
fn rejects_segment_past_eof() {
	let len = synthetic_elf().len() as u64;
	assert_eq!(parse(&mut &with_load(32, 32, 0, 0, len - 8)[..]).unwrap_err(), Error::SegmentOutOfFile);
}

#[test]
fn rejects_self_block_with_missing_phdr() {
	let mut f = self_wrapped(1);
	put(&mut f, 32, 0x800 | (7 << 20), 8); // block type: maps phdr 7 of 2
	assert_eq!(parse(&mut &f[..]).unwrap_err(), Error::SelfSegmentBadMapping);
}

// ---- dynamic (A1–A5) ------------------------------------------------------------------------

const IMAGE: u64 = 0x10000;

struct Module {
	dynamic: Vec<u8>,
	data: Vec<u8>,
}

/// Offsets inside `data`.
const STR_AT: usize = 0;
const STRTAB: &[u8] = b"\0libkernel\0sym_a\0sym_b\0";
const SYM_AT: usize = 32;
const SYMS: usize = 3;
const RELA_AT: usize = SYM_AT + SYMS * SYM_SIZE as usize;
const RELAS: usize = 2;
const JMPREL_AT: usize = RELA_AT + RELAS * RELA_SIZE as usize;

fn module() -> Module {
	let mut data = vec![0u8; JMPREL_AT + RELA_SIZE as usize];
	data[STR_AT..STR_AT + STRTAB.len()].copy_from_slice(STRTAB);
	for (i, name) in [0u64, 11, 17].iter().enumerate() {
		put(&mut data, SYM_AT + i * SYM_SIZE as usize, *name, 4);
	}
	let rela = |data: &mut Vec<u8>, at: usize, offset: u64, r_type: u64, sym: u64| {
		put(data, at, offset, 8);
		put(data, at + 8, (sym << 32) | r_type, 8);
	};
	rela(&mut data, RELA_AT, 0x100, 8, 0); // RELATIVE
	rela(&mut data, RELA_AT + RELA_SIZE as usize, 0x108, 1, 1); // R_X86_64_64 -> sym_a
	rela(&mut data, JMPREL_AT, 0x200, 7, 2); // JUMP_SLOT -> sym_b

	let entries: Vec<(i64, u64)> = vec![
		(tag::OS_STRTAB, STR_AT as u64),
		(tag::OS_STRSZ, STRTAB.len() as u64),
		(tag::OS_SYMTAB, SYM_AT as u64),
		(tag::OS_SYMTABSZ, SYMS as u64 * SYM_SIZE),
		(tag::OS_SYMENT, SYM_SIZE),
		(tag::OS_RELA, RELA_AT as u64),
		(tag::OS_RELASZ, RELAS as u64 * RELA_SIZE),
		(tag::OS_RELAENT, RELA_SIZE),
		(tag::OS_JMPREL, JMPREL_AT as u64),
		(tag::OS_PLTRELSZ, RELA_SIZE),
		(tag::SONAME, 1),
		(tag::OS_NEEDED_MODULE, (1 << 48) | 1),
		(tag::NULL, 0),
	];
	let mut dynamic = Vec::new();
	for (t, v) in entries {
		dynamic.extend_from_slice(&t.to_le_bytes());
		dynamic.extend_from_slice(&v.to_le_bytes());
	}
	Module { dynamic, data }
}

fn set_tag(m: &mut Module, t: i64, v: u64) {
	let i = m.dynamic.as_chunks::<16>().0.iter().position(|e| i64::from_le_bytes(e[0..8].try_into().unwrap()) == t).unwrap();
	put(&mut m.dynamic, i * 16 + 8, v, 8);
}

fn err(m: &Module) -> Code {
	validate(&m.dynamic, &m.data, IMAGE).unwrap_err().code
}

#[test]
fn accepts_valid_module() {
	let m = module();
	assert!(validate(&m.dynamic, &m.data, IMAGE).unwrap().warnings.is_empty());
}

#[test]
fn a5_requires_dt_null() {
	let mut m = module();
	m.dynamic.truncate(m.dynamic.len() - 16);
	assert_eq!(err(&m), Code::NotTerminated);
}

#[test]
fn a1_rejects_relocation_outside_image() {
	for offset in [IMAGE - 4, IMAGE, u64::MAX - 3] {
		let mut m = module();
		put(&mut m.data, RELA_AT, offset, 8);
		assert_eq!(err(&m), Code::RelocOutOfImage, "r_offset {offset:#x}");
	}
	let mut m = module();
	put(&mut m.data, JMPREL_AT, IMAGE, 8);
	assert_eq!(err(&m), Code::RelocOutOfImage);
}

#[test]
fn a2_rejects_tables_outside_data() {
	let mut m = module();
	let end = m.data.len() as u64;
	set_tag(&mut m, tag::OS_RELA, end);
	assert_eq!(err(&m), Code::TableOutOfData);
	let mut m = module();
	set_tag(&mut m, tag::OS_SYMTAB, u64::MAX - 8);
	assert_eq!(err(&m), Code::TableOutOfData);
	let mut m = module();
	let end = m.data.len() as u64;
	set_tag(&mut m, tag::OS_STRSZ, end + 1);
	assert_eq!(err(&m), Code::TableOutOfData);
}

#[test]
fn a2_rejects_bad_entry_sizes() {
	let mut m = module();
	set_tag(&mut m, tag::OS_RELASZ, RELA_SIZE + 1);
	assert_eq!(err(&m), Code::BadEntrySize);
	let mut m = module();
	set_tag(&mut m, tag::OS_SYMENT, 16);
	assert_eq!(err(&m), Code::BadEntrySize);
}

#[test]
fn a2_image_tables_must_fit_image() {
	// Entry 0 becomes an image-resident DT_STRTAB that starts at the image end.
	let mut m = module();
	put(&mut m.dynamic, 0, tag::STRTAB as u64, 8);
	put(&mut m.dynamic, 8, IMAGE, 8);
	assert_eq!(err(&m), Code::TableOutOfImage);
}

#[test]
fn a3_rejects_symbol_index_out_of_range() {
	let mut m = module();
	put(&mut m.data, RELA_AT + RELA_SIZE as usize + 12, SYMS as u64, 4);
	assert_eq!(err(&m), Code::SymbolIndexOutOfRange);
	// RELATIVE does not use its symbol field.
	let mut m = module();
	put(&mut m.data, RELA_AT + 12, 0xffff_ffff, 4);
	assert!(validate(&m.dynamic, &m.data, IMAGE).is_ok());
}

#[test]
fn a4_rejects_names_outside_string_table() {
	let mut m = module();
	put(&mut m.data, SYM_AT + SYM_SIZE as usize, STRTAB.len() as u64, 4);
	assert_eq!(err(&m), Code::NameOutOfRange);
	let mut m = module();
	set_tag(&mut m, tag::SONAME, 1000);
	assert_eq!(err(&m), Code::NameOutOfRange);
	let mut m = module();
	set_tag(&mut m, tag::OS_NEEDED_MODULE, (1 << 48) | 0xffff_0000);
	assert_eq!(err(&m), Code::NameOutOfRange);
}

#[test]
fn a4_name_running_off_data_is_an_error_past_strsz_a_warning() {
	// Shrink STRSZ so "sym_b" crosses its end: its NUL is still inside data -> warning.
	let mut m = module();
	set_tag(&mut m, tag::OS_STRSZ, STRTAB.len() as u64 - 2);
	let report = validate(&m.dynamic, &m.data, IMAGE).unwrap();
	assert_eq!(report.warnings, vec![Finding { code: Code::NamePastStrsz, index: 2 }]);

	// String table at the very end of data with no NUL left: error.
	let mut m = module();
	let at = m.data.len();
	m.data.extend_from_slice(b"abc");
	set_tag(&mut m, tag::OS_STRTAB, at as u64);
	set_tag(&mut m, tag::OS_STRSZ, 3);
	for t in [tag::SONAME, tag::OS_NEEDED_MODULE] {
		set_tag(&mut m, t, 0);
	}
	for i in 0..SYMS {
		put(&mut m.data, SYM_AT + i * SYM_SIZE as usize, 0, 4);
	}
	assert_eq!(err(&m), Code::NameNotTerminated);
}

/// Deterministic fuzzing of tag values and table bytes. Invariants: no panic; on success every
/// relocation target is inside the image and every symbol-using relocation names a real symbol.
#[test]
fn fuzz_dynamic_never_panics_and_ok_means_in_bounds() {
	let base = module();
	let mut state = 0x2545_f491_4f6c_dd1du64;
	let mut next = move || {
		state ^= state << 13;
		state ^= state >> 7;
		state ^= state << 17;
		state
	};
	let extremes = [0u64, 1, 23, 24, IMAGE - 8, IMAGE, 0xffff_ffff, u64::MAX, u64::MAX - 7];
	let (mut ok, mut rejected) = (0u32, 0u32);
	for _ in 0..40_000 {
		let mut m = Module { dynamic: base.dynamic.clone(), data: base.data.clone() };
		for _ in 0..(next() % 4 + 1) {
			let r = next() as usize;
			match r % 3 {
				0 => {
					let at = (r / 3) % (m.dynamic.len() / 16) * 16 + 8;
					put(&mut m.dynamic, at, extremes[(r / 64) % extremes.len()], 8);
				}
				1 => {
					let at = (r / 3) % m.data.len();
					m.data[at] ^= 1 << (r % 8);
				}
				_ => {
					let at = (r / 3) % (m.data.len() - 8);
					put(&mut m.data, at, extremes[(r / 64) % extremes.len()], 8);
				}
			}
		}
		let len = (next() as usize) % (m.dynamic.len() + 1);
		let dynamic = if next() % 8 == 0 { &m.dynamic[..len] } else { &m.dynamic[..] };
		if validate(dynamic, &m.data, IMAGE).is_err() {
			rejected += 1;
		} else {
			ok += 1;
			let get = |t: i64| {
				dynamic.as_chunks::<16>().0.iter().take_while(|e| e[0..8] != [0; 8]).find(|e| i64::from_le_bytes(e[0..8].try_into().unwrap()) == t).map(|e| u64::from_le_bytes(e[8..16].try_into().unwrap()))
			};
			let sym_count = get(tag::OS_SYMTABSZ).unwrap_or(0) / SYM_SIZE;
			for (at_tag, sz_tag) in [(tag::OS_RELA, tag::OS_RELASZ), (tag::OS_JMPREL, tag::OS_PLTRELSZ)] {
				let (Some(at), Some(sz)) = (get(at_tag), get(sz_tag)) else { continue };
				for rec in m.data[at as usize..(at + sz) as usize].as_chunks::<24>().0.iter() {
					let target = u64::from_le_bytes(rec[0..8].try_into().unwrap());
					assert!(target <= IMAGE - 8);
					let r_type = u32::from_le_bytes(rec[8..12].try_into().unwrap());
					let sym = u64::from(u32::from_le_bytes(rec[12..16].try_into().unwrap()));
					if matches!(r_type, 1 | 6 | 7) && get(tag::OS_SYMTAB).is_some() {
						assert!(sym < sym_count);
					}
				}
			}
		}
	}
	// Both paths must be exercised, or the invariants above prove nothing.
	assert!(ok > 1_000 && rejected > 1_000, "ok={ok} rejected={rejected}");
}
