# Windows Kafka (rdkafka + zlib) Linking Notes

## Summary

After enabling Kafka on Windows (MSVC toolchain), the `auth-service` binary failed to link with unresolved `__imp_crc32`, `__imp_deflate*`, and `__imp_inflate*` symbols from `librdkafka` objects. These indicate the linker expected the **zlib import library** (`zlib.lib` / `zlibd.lib`) but it was missing from the link line.

## Root Cause

`rdkafka-sys` (CMake + vcpkg) did build and stage zlib under its private tree:

```text
target/<profile>/build/rdkafka-sys-<hash>/out/build/vcpkg_installed/x64-windows/{lib,debug/lib}/
```

Cargo did not emit a `cargo:rustc-link-search` or `cargo:rustc-link-lib=zlib` for that location, so the final link step never saw the import library.

## Failed Attempts (Chronology)

1. Added `libz`, `libz-static` rdkafka features plus env vars `LIBZ_SYS_STATIC=1`, `ZLIB_STATIC=1`.
2. Forced internal build with `RDKAFKA_BUILD_ZLIB=1`.
3. Added `.cargo/config.toml` rustflags `/LIBPATH:` pointing at a `libz-sys` output.
4. Emitted `cargo:rustc-link-lib=static=z` (mismatched naming vs import lib) via early build script.
5. Removed/added explicit `libz-sys` dependencies multiple times.

All failed because none injected the actual vcpkg zlib import library directory for the final crate link.

## Working Solution

Introduce a crate-local `build.rs` (currently in `auth-service`) that:

- Scans `target/<profile>/build/rdkafka-sys-*` directories (sorted by modified time) to find the most recent.
- Searches for `zlibd.lib` (debug) or `zlib.lib` (release) under `.../vcpkg_installed/x64-windows/`.
- Emits:
  - `cargo:rustc-link-search=native=<that dir>`
  - `cargo:rustc-link-lib=zlib`

Result: MSVC linker resolves zlib symbols; build succeeds.

### Why Not Just `static=z`?

The unresolved names were `__imp_*` thunks (import address table). That means the build expected a DLL import library, not a purely static archive. The name `z` (used on some GNU toolchains) did not map to the produced `zlib.lib` on Windows.

## Current State

Build now succeeds. A follow‑up will harden the script (see below) and replicate logic for other Kafka-using services if/when needed.

## Actionable Steps for New Kafka Service on Windows

1. Add dependency (minimal feature set):

   ```toml
   rdkafka = { version = "0.29", default-features = false, features = ["cmake-build", "tokio", "libz", "libz-static"] }
   ```

2. Copy (or abstract) the `build.rs` logic OR depend on a shared internal crate once created.
3. Build verbosely to verify: `cargo build -p your-service -vv` (confirm `-l zlib`).

## Clean Rebuild (Troubleshooting)

```powershell
cargo clean -p auth-service
cargo build -p auth-service -vv
```

Check that the link arguments contain both `-L ...vcpkg_installed\x64-windows\lib` and `-l zlib`.

## Troubleshooting Matrix

| Symptom | Likely Cause | Resolution |
|---------|--------------|-----------|
| `__imp_crc32` unresolved | Missing import lib on link line | Ensure build script emits link-search + `-l zlib` |
| `link.exe` cannot open `zlib.lib` | Path incorrect or build cleaned | Re-run build with `-vv`, verify directory exists |
| Multiple `libz-sys-*` dirs | Normal (hash per build) | Harmless; only search path with import lib matters |
| Build script seemingly ignored | Cargo cache not invalidated | Touch `build.rs` or `cargo clean -p <crate>` |

## Environment Variables No Longer Required

These were exploratory and can be removed (validate in CI):

`LIBZ_SYS_STATIC`, `ZLIB_STATIC`, `RDKAFKA_BUILD_ZLIB`.

## Future Hardening (Planned)

- Distinguish debug vs release and prefer `zlibd.lib` in debug.
- Emit `cargo:warning=linked zlib from <path>` for traceability.
- Panic early with a clear message if no import library is found.
- Extract shared helper crate (e.g. `build-support-kafka-link`) for reuse.
- Add Windows CI matrix job building one Kafka consumer/producer binary.

## Reference Errors (Pre-Fix)

```text
error LNK2019: unresolved external symbol __imp_crc32
error LNK2019: unresolved external symbol __imp_deflate
error LNK2019: unresolved external symbol __imp_inflate
```

## Verification

After applying `build.rs`, `cargo build -p auth-service -vv` shows:

```text
cargo:rustc-link-search=native=...\rdkafka-sys-<hash>\out\build\vcpkg_installed\x64-windows\lib
cargo:rustc-link-lib=zlib
```

And the link step finishes without unresolved symbols.

---

Last updated: 2025-10-01
