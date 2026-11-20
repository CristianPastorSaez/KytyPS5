# DEVLOG

Technical log of this fork. Newest entry first. Each entry: what was measured or changed, the
exact numbers, open bottlenecks, next steps. Raw data stays in `_Build/bench/` (git-ignored).

---

## 2026-11-20 — `kyty_elf` phase 2: segments, dynamic section, runtime-linker hardening

Audit of `elf.cpp` + `runtimeLinker.cpp` (file-controlled values used as offset/index/size
without checking the buffer they point into):

| # | Where | Problem |
|---|---|---|
| A1 | `runtimeLinker.cpp` `RelocateRecord` → `PatchGuestMemory64` | 8-byte write at `base_vaddr + r_offset`, no range check: **arbitrary write into the emulator process** |
| A2 | `ParseProgramDynamicInfo`, `Elf64::GetDynamicData` | `DT_OS_*` tables and sizes never checked against `PT_OS_DYNLIBDATA` |
| A3 | `GetRelocationInfo` | relocation symbol index not checked against the symbol count |
| A4 | symbol/SONAME/module/library names | name offsets unchecked, termination unchecked |
| A5 | `Elf64::GetDynValue` / `GetDynList` | walk to `DT_NULL` with no bound |
| A6 | `Elf64::LoadSegment` (SELF) | block's `phdr_id` indexes `m_phdr` unchecked |
| A7 | `CalcBaseSize`, PT_LOAD loop | `p_filesz > p_memsz`, `p_vaddr + memsz` overflow, non-power-of-two `p_align` |
| A8 | `LoadSegment` (plain ELF) | segment file range unchecked |

Rust (`rust/kyty_elf`):
- `parse()` now also checks every PT_LOAD/RELRO/DYNAMIC/DYNLIBDATA segment (A7, A8) and every
  SELF block mapping (A6).
- New `dynamic::validate(dynamic, dynlibdata, image_size)` (A1–A5): `DT_NULL` present; `DT_OS_*`
  tables inside dynlibdata, `DT_*` tables inside the image; entry sizes; every relocation target
  in `[0, image_size - 8]`; symbol indices < symbol count; every name offset < `STRSZ` and
  NUL-terminated inside dynlibdata.
- Rule: memory-safety violations are errors; in-bounds inconsistencies are warnings (a name
  terminating past `STRSZ` but inside dynlibdata) and only get logged.
- FFI: `kyty_elf_validate_dynamic(...)` → status + `KytyElfDynamicReport` (failing entry index,
  warning count/first warning).

C++ (independent of Rust, always compiled):
- `RelocateRecord`: refuses any target outside `[base_vaddr, base_vaddr + base_size - 8]`.
- `GetDynValue`/`GetDynList`: bounded by the `PT_DYNAMIC` size (now stored in `Elf64`).
- `LoadSegment`: SELF block must map an existing program header.
- These use `if (...) EXIT(...)`, not `EXIT_IF`, which compiles away under `KYTY_FINAL`.

Verification:
- `cargo test`: 26 tests (16 new: one per audit row plus edge cases); the dynamic fuzz test runs
  40 k mutations of tag values and table bytes and asserts both outcomes occur > 1,000 times so
  its in-bounds invariants are actually exercised. `cargo clippy --all-targets`: clean.
- CI set + `kyty_elf_rust`: 33/33.
- End to end (homebrew `hello_world`): valid → boots, no warnings. `r_offset` patched to
  `0x7fff_ffff_0000` → `invalid dynamic section (relocation target outside module image,
  entry 0)`. Symbol `st_name` past `STRSZ` → `invalid dynamic section (name offset past string
  table size, entry 1)`. Both exit cleanly.
- Same A1 file on a build with `-DKYTY_RUST_ELF=OFF`: the C++ check alone stops it
  (`relocation target 0x00008008ffff0000 outside module image`); before, that was an 8-byte
  write to an arbitrary host address.

Not covered: contents of image-resident `DT_*` tables (only their ranges; PS5 titles use
`DT_OS_*`), the hash table's contents (no consumer found).

---

## 2026-11-20 — Presentation loop at idle (Tracy) — earlier GPU claim retracted

Built `tracy-capture` and `tracy-csvexport` 0.14.1 (same version as the client in
`3rdparty/tracy`) into `_Build/`. Run: homebrew `hello_world` (never opens VideoOut),
`--profile --no-splash`, 25 s capture, `nvidia-smi pmon` filtered to the emulator PID.

- **Correction:** the baseline's "GPU 30 % vs 4.5 % idle" compared system-wide utilisation at
  two moments. Per-process `pmon` shows Chrome alone swinging 2–46 % SM on this machine. The
  emulator's own share was `-` (below threshold) in one 8 s window and 10.5 % mean (21.9 % when
  sampled) in another. Its GPU cost at idle is real but small and noisy; "25 points" is wrong.
- Tracy: `VideoOutDriver::Impl::PresentThread` presents a **blank 1280×720 frame on every
  vblank** — 1,507 × `Presenter::Present` + `PrepareBlankFrame` in 25 s (60 Hz) although the
  guest has no VideoOut port open. CPU cost: `Present` 142 µs mean (max 14.8 ms, first
  frames), `PrepareBlankFrame` 41 µs → ≈0.9 % of one core.
- Source (`videoOut.cpp:822-839`): blank frames are presented when the system overlay needs a
  refresh (`NeedsSystemOverlayRefresh`) or on a late vblank with no port open. Which branch
  fires here is not yet determined (no counter or zone separates them).

Status: low priority. A "present only on change" idle path would save a 60 Hz clear+blit, but
the overlay may depend on it. Next: separate the two branches (Tracy zone/plot), then GPU
timestamps if the cost justifies it.

---

## 2026-11-20 — Module #2: `kyty_elf` Rust crate (ELF/SELF header tables)

Problem: `elf.cpp` read the SELF segment table, program/section header tables and the
section-name string table with `Seek(offset)` + `Read(count * size)` straight from header
fields — no check that the range fits the file, unchecked short reads (uninitialised structs),
and a string table that might not be NUL-terminated.

Design: `rust/kyty_elf` (no dependencies). C++ keeps the file I/O (read callback over
`Common::File`) and the semantic checks (`IsSelf`, `IsValid` with their logs); Rust reads every
table only after checking `offset + count * size` against the file size with overflow-safe
arithmetic, rejects string tables > 4 GiB or not NUL-terminated, and returns raw bytes that C++
`memcpy`s into its structs (`static_assert`ed sizes). Fields are decoded with `from_le_bytes`;
`unsafe` exists only in `ffi.rs`. CMake: `KYTY_RUST_ELF` (ON, falls back to C++ without cargo),
cargo builds into `<build>/rust`, tests registered as ctest `kyty_elf_rust`.

Verification:
- `cargo test`: 10 tests — valid ELF, SELF-wrapped ELF, table past EOF, `phoff` overflow, bad
  string table, SELF segments past EOF, unknown SELF tail, too small, and a deterministic fuzz
  test (all truncations + 40 k mutations of offset/count fields): no panics, every returned
  table matches its header. `cargo clippy --all-targets`: clean.
- CI set 32/32 with the Rust reader linked into `kyty_emulator`.
- Homebrew `hello_world` boots exactly as before (`main: Hello world!`).
- Same homebrew with `e_phnum = 0xFFFF`: `ELF: ...: program header table out of file bounds`,
  then a controlled `elf is not valid` exit instead of reading past the file.

Behaviour change: a structurally broken file is now rejected before `IsValid` runs, so its log
names the out-of-range table instead of an `IsValid` reason. Out of scope (next): the
`PT_DYNAMIC` / `PT_OS_DYNLIBDATA` loads through `LoadSegment`, and dynamic-table parsing in
`runtimeLinker.cpp`.

---

## 2026-11-20 — Patch #1: lazy region lookup tables (`fix/lazy-page-tables`, `96986ee6`)

`MemoryTracker` and `PageManager::Impl` replaced `std::make_unique<std::atomic<T*>[]>(2^22)`
(value-initialised → 32 MiB written per table) with `RegionTable<T>` in `regionDefinitions.h`:
`calloc`'d `T*[]` accessed through `std::atomic_ref` (same acquire/release). Microbenchmark of
one 32 MiB table: 6,000 µs / 8,212 faults → 21 µs / 4 faults (`VirtualAlloc`: 15 µs / 4).

Before/after (medians, `run-synthetic.ps1 -Seconds 30`, exact exit counters, first run of new
binaries discarded):

| Suite | Wall | Page faults | Peak working set | Peak committed |
|---|---|---|---|---|
| memory_tracker_tests | 0.364 → **0.090 s** (−75 %) | 280,618 → **1,612** (−99.4 %) | 68.9 → **5.3 MiB** | 73.1 → 73.1 MiB |
| page_manager_tests | 0.230 → **0.118 s** (−49 %) | 75,336 → **1,484** (−98 %) | 36.9 → **5.2 MiB** | 45.0 → 45.0 MiB |

Committed memory is unchanged by design (calloc commits, the OS only backs touched pages).
CI set 32/32, `shader_recompiler_compute_tests` passes. In the emulator this saves 64 MiB of
resident memory and ~16 k faults at boot; in the test suites, per constructed context.
Baseline binaries kept in `_Build/bench/bin-a1fc5ae4/` for re-runs.

---

## 2026-11-20 — Environment, baseline, first profile

Base: upstream `a1fc5ae4`. Host: i7-13700KF (24 threads), 32 GB, RTX 3060 (driver 610.88,
Vulkan 1.4.341), Windows 11. Toolchain: VS 2022 Build Tools clang-cl 19.1.5, CMake 4.4.4,
Ninja 1.12.1, Vulkan SDK 1.4.363.0, Qt 6.10.3, Rust 1.98.1.

### Build and CI parity
- Release build OK. CI test set (32 suites): **32/32 pass, 40 s** first run.

### Synthetic run (`tools/bench/run-synthetic.ps1`, 250 ms sampling)
| Suite | Wall | CPU | Peak private | Page faults |
|---|---|---|---|---|
| shader_recompiler_compute | 91.1 s | 61.9 s (≈0.7 of 24 cores) | 8.9 GB | 20.5 M |
| memory_tracker | 0.63 s | 0.23 s | 33 MB | 280 k |
| page_manager | 0.31 s | 0.08 s | 33 MB | 23 k |
| others (VMA, archive, save_data, resource_*) | ≈0.3 s each | — | < 50 MB | 1–7 k |

GPU during the run: 11 % avg, 4.9 GB peak VRAM.

### ETW profile of shader_recompiler_compute (WPR CPU sampling with stacks, ≈18.6 k samples)
Inclusive CPU share:
| Path | % | In a real game? |
|---|---|---|
| `RenderContext` rebuilt per test case (`~BufferCache` → VMA `FreeDedicatedMemory` → driver 18 %, `BufferCache` ctor allocating ~770 MiB of fixed buffers 5 %) | 26 % | boot only |
| SPIR-V validation (`spvtools::SpirvTools::Validate`) | 20 % | no — gated by `--shader-validation` (`pipelineCache.cpp:137`) |
| test `printf` logging | 12 % | no |
| harness buffers (`VulkanHarness::CreateStorageBuffer`) | 7 % | no |
| `MemoryTracker` + `PageManager::Impl` ctors (`memset` of two 32 MiB tables) | 5.7 % | **yes** |
| RDNA2→SPIR-V recompiler | ≈2 % | yes |
| `kernel/memory.cpp` guest memory | 0.4 % | yes |

**Conclusion:** this suite mostly measures its own scaffolding; the recompiler and guest-memory
code are not the bottleneck there. The 20.5 M page faults come from per-case allocate/zero/free
of hundreds of MiB.

### ctest "contention" — not emulator code
First ctest run: `resource_tracking` 24 s, `save_data_memory` 14.5 s; alone 0.3 s. Reproduced:
only **freshly linked** binaries are slow on their first launch (5–26×, e.g. `ajm_resample`
1.84 s → 0.07 s); a second pass of the same binaries: whole set 0.7 s at `-j12`. Stacks show
synchronous writes through `fltMgr.sys` (on-access scanning). Not a code issue.

### Homebrew boot (OpenOrbis v0.5.4 `hello_world`, built locally, no Sony `.prx` used)
- PS4 fake-SELF is rejected by design: `runtimeLinker.cpp:1651` (`!is_next_gen`).
- Test artefact: the `.oelf` with `EI_ABIVERSION` patched 0 → 2 loads as "next-gen" and runs to
  `main: Hello world!`. 8 libkernel imports unresolved (stubbed); `BPE9s9vQQXo` is hit during
  `printf`.
- 60 s monitored: steady state in < 3 s; 1.8 GB private, 767 MB working set, 222 k page faults
  (all during boot), 30 threads. ~~GPU 30 % with nothing drawn vs 4.5 % idle desktop~~
  **Retracted** — system-wide GPU % taken at different moments; Chrome alone swings 2–46 %. See
  the "Presentation loop" entry.

### Real bottlenecks (measured and cross-checked with source)
1. `MemoryTracker` (`memoryTracker.cpp:11`) and `PageManager::Impl` (`pageManager.cpp:154`)
   value-initialise a 32 MiB `std::atomic<T*>[2^22]` table each (16 TiB tracked space / 4 MiB
   regions) → 64 MiB `memset` + page faults per construction, though only touched regions are
   ever used. → **Patch #1.**
2. Guest direct memory is a 12 GiB `SEC_COMMIT` section committed at boot
   (`memoryAddressSpace.inc:40`); fails on small page files. Commit-on-demand is a larger change.
3. ~~Present/vblank loop costs ~25 GPU points with no guest work~~ — retracted, see the
   "Presentation loop" entry.
4. ELF/SELF loader reads header tables without bounds checks (`elf.cpp:24-125`) → robustness /
   security, candidate for the Rust `kyty_elf` crate.

Not measurable with this baseline: asset streaming and runtime shader-compile stutter in real
open-world games (#1937) — needs a real title run locally with Tracy.

### Next
- ~~Patch #1~~ done (see entry above).
- ~~`kyty_elf` Rust crate~~ done (see entry above). Next: dynamic segment loads + runtime linker tables.
