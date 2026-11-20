#ifndef KYTY_ELF_H_
#define KYTY_ELF_H_

/* C ABI of the Rust kyty_elf crate: bounds-checked SELF/ELF header table reader.
 * Every table returned lies entirely inside the file; semantic validation (magic, machine,
 * ABI version) is left to the caller. */

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct KytyElf KytyElf;

/* Reads `size` bytes at `offset` into `dst`; returns non-zero only if all bytes were read. */
typedef int32_t (*KytyElfReadFn)(void* ctx, uint64_t offset, uint8_t* dst, uint64_t size);

enum {
	KYTY_ELF_TABLE_SELF_HEADER   = 0, /* 32 bytes when the file has at least 32 */
	KYTY_ELF_TABLE_SELF_SEGMENTS = 1, /* SELF files only */
	KYTY_ELF_TABLE_EHDR          = 2,
	KYTY_ELF_TABLE_PHDRS         = 3, /* empty when the ELF magic is wrong */
	KYTY_ELF_TABLE_SHDRS         = 4, /* plain ELF files only */
	KYTY_ELF_TABLE_STRTAB        = 5, /* NUL-terminated when present */
};

/* 0 on success (free *out with kyty_elf_free), else an error code for kyty_elf_error_name. */
int32_t     kyty_elf_open(void* ctx, KytyElfReadFn read, uint64_t file_size, KytyElf** out);
void        kyty_elf_free(KytyElf* elf);
/* Raw table bytes valid until kyty_elf_free, or null with *size = 0 when absent. */
const uint8_t* kyty_elf_table(const KytyElf* elf, int32_t table, uint64_t* size);
int32_t     kyty_elf_is_self(const KytyElf* elf);
uint64_t    kyty_elf_ehdr_offset(const KytyElf* elf);
const char* kyty_elf_error_name(int32_t code);

/* Outcome of kyty_elf_validate_dynamic. Error codes are memory-safety violations; warnings
 * (first_warning != 0) are in-bounds inconsistencies worth logging. */
typedef struct KytyElfDynamicReport {
	int32_t  error;       /* 0 or the error code (also the return value) */
	uint64_t error_index; /* dynamic entry, symbol or relocation that failed */
	uint32_t warnings;
	int32_t  first_warning;
	uint64_t first_warning_index;
} KytyElfDynamicReport;

/* Validates PT_DYNAMIC (`dynamic`), PT_OS_DYNLIBDATA (`data`) and every table, relocation target
 * (must lie in [0, image_size - 8]), symbol index and name the runtime linker will use.
 * Pointers may be null only when their size is 0. `report` may be null. */
int32_t kyty_elf_validate_dynamic(const uint8_t* dynamic, uint64_t dynamic_size, const uint8_t* data,
                                  uint64_t data_size, uint64_t image_size,
                                  KytyElfDynamicReport* report);

#ifdef __cplusplus
}
#endif

#endif /* KYTY_ELF_H_ */
