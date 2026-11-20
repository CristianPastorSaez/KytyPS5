# CLAUDE.md — working rules for this fork

Instructions for any AI assistant (local Claude Code or claude.ai with GitHub access) working
on this repository. Measurements and history live in [`DEVLOG.md`](DEVLOG.md); read its latest
entry before starting.

## What this fork is

A personal fork of [KytyPS5](https://github.com/KytyPS5/KytyPS5) (PS5 translation layer, C++20,
Vulkan 1.3, GPL-2.0). Goal: find and fix real memory / I/O bottlenecks with measured
before/after numbers, and add memory-safe Rust modules where untrusted data is parsed.

## Branches

| Branch | Purpose | May touch README / CLAUDE.md / DEVLOG.md? |
|---|---|---|
| `main` | Mirror of `upstream/main`. Never commit here. | — |
| `dev/ps5-architecture` | Fork work, experiments, docs, bench tools. | Yes |
| `fix/<topic>`, `feat/<topic>` | One upstream PR each, cut from `main`. | **No** — code + tests only |
| `feat/upstream-ready` | Template/placeholder for upstream work. | No |

Sync: `git switch main && git pull --ff-only` (main tracks `upstream/main`, push to upstream is
disabled locally), then rebase or merge `main` into the working branch.
Commits stay local until the owner explicitly approves each push.

## Hard rules

- **Clean-room only.** Never generate, request, download or reference Sony keys, firmware,
  original `.prx`/`.sprx` modules or commercial game dumps. Test with synthetic suites or
  open-source homebrew built locally (OpenOrbis samples ship Sony `.prx` files in
  `sce_module/` — never use them).
- Measure before optimizing; every perf change states before/after numbers from the same tools.
- Upstream-bound branches follow upstream style (clang-format via `pre-commit`) and add no
  fork-specific files.

## Code conventions

- **C++**: C++20, Clang/clang-cl only (MSVC `cl.exe` unsupported), upstream `.clang-format`.
  Keep platform splits as `#if KYTY_PLATFORM == KYTY_PLATFORM_WINDOWS / ... LINUX` like the
  surrounding code (macOS rides the Linux path).
- **Rust** (new modules): one crate per subsystem under `rust/<crate>/`, `crate-type =
  ["staticlib", "rlib"]`, C ABI (`#[no_mangle] extern "C"`) with a hand-written header in
  `rust/<crate>/include/`, no `unsafe` outside `ffi.rs`, `panic = "abort"` in release (never
  unwind into C++), `cargo test` incl. a deterministic fuzz test for any parser, `cargo clippy`
  clean, `Cargo.lock` committed (builds use `--locked`). Each crate ships a `<crate>.cmake` that
  builds it with cargo, exposes an IMPORTED target and registers its tests with ctest; a
  `KYTY_RUST_*` option falls back to the C++ path when cargo is missing.
- Existing Rust modules: `rust/kyty_elf` — bounds-checked SELF/ELF header table reader used by
  `Loader::Elf64::Open` (`KYTY_RUST_ELF`, default ON). C++ keeps the file I/O and `IsValid`.

## Build (Windows)

Prereqs: VS 2022 Build Tools with *C++ Clang tools*, Vulkan SDK (glslang), Qt 6.10.3
msvc2022_64 at `C:\Qt`, `git submodule update --init --recursive`.

```bat
tools\build-windows.cmd --target launcher   :: env: KYTY_BUILD_DIR, KYTY_EXTRA_CMAKE
```

Or, from an x64 Native Tools prompt with VS's `clang-cl` first on `PATH`
(a WinLibs/MinGW `clang-cl` earlier on PATH breaks the build):

```bat
cmake -S . -B _Build/windows -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl -DCMAKE_PREFIX_PATH="C:/Qt/6.10.3/msvc2022_64"
cmake --build _Build/windows --target launcher
```

Tests are `EXCLUDE_FROM_ALL`: build them by name (see `.github/workflows/build.yml`), then
`ctest --test-dir _Build/windows -R "^(name|...)$"`.

## Measuring

- `tools/bench/run-synthetic.ps1` — runs suites (or `-Exe` any program) for N seconds, sampling
  CPU, memory, page faults, I/O and GPU into `_Build/bench/<timestamp>/`.
- `tools/bench/ctest-contention.ps1` — serial vs parallel ctest with system counters.
- `tools/bench/wpr-capture.ps1` — ETW CPU sampling with stacks (needs admin); analyse with
  `xperf -i x.etl -o out.txt -symbols -a profile -detail` or `-a stack -butterfly -pid <pid>`.
  Profile a symbolized copy built in `_Build/windows-prof` (Release + `/Z7` + `/DEBUG:FULL`).
- Gotcha: the first run of a freshly linked `.exe` is 5–26× slower (on-access scanning);
  discard first runs when timing.
