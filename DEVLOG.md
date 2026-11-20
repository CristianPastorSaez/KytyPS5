# DEVLOG

Technical log of this fork. Newest entry first. Each entry: what was measured or changed, the
exact numbers, open bottlenecks, next steps. Raw data stays in `_Build/bench/` (git-ignored).

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
  (all during boot), 30 threads. **GPU 30 % with nothing drawn vs 4.5 % idle desktop** (+1 GB
  VRAM).

### Real bottlenecks (measured and cross-checked with source)
1. `MemoryTracker` (`memoryTracker.cpp:11`) and `PageManager::Impl` (`pageManager.cpp:154`)
   value-initialise a 32 MiB `std::atomic<T*>[2^22]` table each (16 TiB tracked space / 4 MiB
   regions) → 64 MiB `memset` + page faults per construction, though only touched regions are
   ever used. → **Patch #1.**
2. Guest direct memory is a 12 GiB `SEC_COMMIT` section committed at boot
   (`memoryAddressSpace.inc:40`); fails on small page files. Commit-on-demand is a larger change.
3. Present/vblank loop costs ~25 GPU points with no guest work → investigate with Tracy.
4. ELF/SELF loader reads header tables without bounds checks (`elf.cpp:24-125`) → robustness /
   security, candidate for the Rust `kyty_elf` crate.

Not measurable with this baseline: asset streaming and runtime shader-compile stutter in real
open-world games (#1937) — needs a real title run locally with Tracy.

### Next
- Patch #1: lazy OS-zeroed tables, before/after on `memory_tracker_tests` / `page_manager_tests`.
- Then: `kyty_elf` Rust crate.
