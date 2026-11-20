use super::*;

/// Synthetic next-gen ELF: header, 2 program headers, 3 section headers, string table.
pub(crate) fn synthetic_elf() -> Vec<u8> {
	let strtab = b"\0.text\0.shstrtab\0";
	let phoff = EHDR_SIZE;
	let shoff = phoff + 2 * PHDR_SIZE;
	let stroff = shoff + 3 * SHDR_SIZE;
	let mut f = vec![0u8; (stroff as usize) + strtab.len()];
	f[0..4].copy_from_slice(b"\x7fELF");
	f[4] = 2; // ELFCLASS64
	f[5] = 1; // little-endian
	f[6] = 1;
	f[7] = 9; // FreeBSD
	f[8] = 2; // next-gen
	f[16..18].copy_from_slice(&0xfe10u16.to_le_bytes());
	f[18..20].copy_from_slice(&62u16.to_le_bytes());
	f[32..40].copy_from_slice(&phoff.to_le_bytes());
	f[40..48].copy_from_slice(&shoff.to_le_bytes());
	f[54..56].copy_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
	f[56..58].copy_from_slice(&2u16.to_le_bytes());
	f[58..60].copy_from_slice(&(SHDR_SIZE as u16).to_le_bytes());
	f[60..62].copy_from_slice(&3u16.to_le_bytes());
	f[62..64].copy_from_slice(&2u16.to_le_bytes()); // shstrndx
	let sh2 = (shoff + 2 * SHDR_SIZE) as usize;
	f[sh2 + 24..sh2 + 32].copy_from_slice(&stroff.to_le_bytes());
	f[sh2 + 32..sh2 + 40].copy_from_slice(&(strtab.len() as u64).to_le_bytes());
	f[stroff as usize..].copy_from_slice(strtab);
	f
}

fn parse_bytes(b: &[u8]) -> Result<Layout, Error> {
	parse(&mut &b[..])
}

fn put_u16(f: &mut [u8], at: usize, v: u16) {
	f[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u64(f: &mut [u8], at: usize, v: u64) {
	f[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn parses_valid_elf() {
	let l = parse_bytes(&synthetic_elf()).unwrap();
	assert!(!l.is_self);
	assert_eq!(l.ehdr_offset, 0);
	assert_eq!(l.phdrs.len() as u64, 2 * PHDR_SIZE);
	assert_eq!(l.shdrs.len() as u64, 3 * SHDR_SIZE);
	assert_eq!(l.strtab.as_deref(), Some(&b"\0.text\0.shstrtab\0"[..]));
}

#[test]
fn rejects_tables_past_end_of_file() {
	let mut f = synthetic_elf();
	put_u16(&mut f, 56, 1000); // phnum
	assert_eq!(parse_bytes(&f).unwrap_err(), Error::PhdrsOutOfRange);

	let mut f = synthetic_elf();
	let eof = f.len() as u64;
	put_u64(&mut f, 40, eof); // shoff at EOF
	assert_eq!(parse_bytes(&f).unwrap_err(), Error::ShdrsOutOfRange);
}

#[test]
fn rejects_offset_overflow() {
	let mut f = synthetic_elf();
	put_u64(&mut f, 32, u64::MAX - 8); // phoff + phnum * 56 wraps
	assert_eq!(parse_bytes(&f).unwrap_err(), Error::PhdrsOutOfRange);
}

#[test]
fn rejects_bad_string_table() {
	let mut f = synthetic_elf();
	let last = f.len() - 1;
	f[last] = b'x';
	assert_eq!(parse_bytes(&f).unwrap_err(), Error::StrtabNotTerminated);

	let mut f = synthetic_elf();
	let sh2 = (EHDR_SIZE + 2 * PHDR_SIZE + 2 * SHDR_SIZE) as usize;
	put_u64(&mut f, sh2 + 32, u64::MAX); // sh_size
	assert_eq!(parse_bytes(&f).unwrap_err(), Error::StrtabOutOfRange);
}

#[test]
fn non_elf_magic_reads_no_tables() {
	let mut f = synthetic_elf();
	f[0] = 0;
	let l = parse_bytes(&f).unwrap();
	assert!(l.phdrs.is_empty() && l.shdrs.is_empty() && l.strtab.is_none());
}

#[test]
fn too_small() {
	assert_eq!(parse_bytes(&[0u8; 10]).unwrap_err(), Error::TooSmall);
}

pub(crate) fn self_wrapped(segments: u16) -> Vec<u8> {
	let mut f = vec![0u8; SELF_HEADER_SIZE as usize];
	f[0..12].copy_from_slice(&[0x4f, 0x15, 0x3d, 0x1d, 0x00, 0x01, 0x01, 0x12, 0x01, 0x01, 0x00, 0x00]);
	put_u16(&mut f, 24, segments);
	put_u16(&mut f, 26, 0x22);
	f.extend(vec![0u8; usize::from(segments) * SELF_SEGMENT_SIZE as usize]);
	f.extend(synthetic_elf());
	f
}

#[test]
fn self_header_moves_ehdr_and_skips_sections() {
	let l = parse_bytes(&self_wrapped(3)).unwrap();
	assert!(l.is_self);
	assert_eq!(l.ehdr_offset, SELF_HEADER_SIZE + 3 * SELF_SEGMENT_SIZE);
	assert_eq!(l.self_segments.len() as u64, 3 * SELF_SEGMENT_SIZE);
	assert_eq!(l.phdrs.len() as u64, 2 * PHDR_SIZE);
	assert!(l.shdrs.is_empty() && l.strtab.is_none());
}

#[test]
fn self_segments_past_end_of_file() {
	let mut f = self_wrapped(1);
	put_u16(&mut f, 24, u16::MAX);
	assert_eq!(parse_bytes(&f).unwrap_err(), Error::SelfSegmentsOutOfRange);
}

#[test]
fn unknown_self_tail_is_plain_file() {
	let mut f = self_wrapped(1);
	put_u16(&mut f, 26, 0x99); // magic matches, tail does not: C++ IsSelf says no
	let l = parse_bytes(&f).unwrap();
	assert!(!l.is_self && l.self_header.is_some());
}

/// Deterministic fuzzing without external crates: truncations, bit flips and extreme values in
/// the offset/count fields. Invariants: never panics, and every returned table has exactly the
/// size its header declares.
#[test]
fn fuzz_never_panics_and_tables_match_headers() {
	let seeds = [synthetic_elf(), self_wrapped(2)];
	let mut state = 0x9e37_79b9_7f4a_7c15u64;
	let mut next = move || {
		state ^= state << 13;
		state ^= state >> 7;
		state ^= state << 17;
		state
	};
	let fields = [24usize, 26, 32, 40, 56, 60, 62]; // SELF count/tail; ehdr phoff/shoff/phnum/shnum/shstrndx
	let extremes = [0u64, 1, 0xffff, 0x7fff_ffff, u64::MAX, u64::MAX - 63];
	for seed in &seeds {
		for len in 0..=seed.len() {
			let _ = parse_bytes(&seed[..len]);
		}
		for _ in 0..20_000 {
			let mut f = seed.clone();
			for _ in 0..(next() % 4 + 1) {
				let r = next() as usize;
				if r.is_multiple_of(2) {
					let at = r / 2 % f.len();
					f[at] ^= 1 << (r % 8);
				} else {
					let base = if seed.len() > synthetic_elf().len() { (SELF_HEADER_SIZE + 2 * SELF_SEGMENT_SIZE) as usize } else { 0 };
					let at = base + fields[r / 2 % fields.len()];
					let v = extremes[(r / 16) % extremes.len()].to_le_bytes();
					let end = (at + 8).min(f.len());
					f[at..end].copy_from_slice(&v[..end - at]);
				}
			}
			if let Ok(l) = parse_bytes(&f) {
				if l.ehdr[0..4] == *b"\x7fELF" {
					assert_eq!(l.phdrs.len() as u64, u64::from(u16_at(&l.ehdr, 56)) * PHDR_SIZE);
					if !l.is_self {
						assert_eq!(l.shdrs.len() as u64, u64::from(u16_at(&l.ehdr, 60)) * SHDR_SIZE);
					}
				}
				if let Some(s) = &l.strtab {
					assert_eq!(s.last(), Some(&0));
				}
			}
		}
	}
}
