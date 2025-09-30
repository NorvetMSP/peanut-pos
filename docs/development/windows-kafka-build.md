# Windows Build Notes for Kafka (rdkafka)

Windows linking errors (e.g. `__imp_crc32`, `__imp_deflate`, `__imp_inflate`) occur when `rdkafka` cannot find zlib symbols. The fix we applied:

## Changes Implemented

- Added `libz-static` feature to every `rdkafka` dependency line.
- Disabled default features where not already disabled and explicitly enabled `tokio` + `cmake-build`.
- Added a Windows-only dependency block:

  ```toml
  [target.'cfg(target_os = "windows")'.dependencies]
  libz-sys = { version = "1.1.22", features = ["static"] }
  ```

## Why This Works

`librdkafka` depends on zlib for compression. On Linux the system zlib is available in the build image. On Windows (MSVC toolchain) static linking requires explicitly pulling in `libz-sys` with `static` feature; the `libz-static` rdkafka feature wires that up, but some crates still need the explicit `libz-sys` stanza for consistent resolution across all service crates.

## Clean Rebuild Steps (Windows)

```powershell
# From repo root
cargo clean
# (Optional) ensure you have a recent CMake and Visual Studio Build Tools installed
cargo build -p auth-service
```

If you still see unresolved symbols, verify that `vcpkg` or system-wide zlib is not interfering; ensure no stale build artifacts remain under `target/`.

## Adding a New Kafka Service

1. Add dependency:

   ```toml
   rdkafka = { version = "0.29", default-features = false, features = ["cmake-build", "tokio", "libz-static"] }
   ```

2. Ensure the Windows block (above) exists or add `libz-sys` if absent.
3. Run a quick build:

   ```powershell
   cargo build -p your-new-service
   ```

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `__imp_crc32` unresolved | Missing static zlib | Ensure `libz-static` + `libz-sys` static feature present |
| `link.exe` cannot find `zlib.lib` | Visual Studio build tools/zlib not found | Install VS Build Tools, keep using static feature to embed |
| Build hangs on `cmake` step | Missing CMake | Install CMake or add it to PATH |

## Future Improvements

- Introduce a shared `[workspace.dependencies]` section (Rust 1.64+) to centralize rdkafka+zlib settings.
- Add a CI matrix job (Windows) that builds one Kafka service to guard against regressions.
