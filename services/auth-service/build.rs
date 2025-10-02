use std::{env, fs, path::PathBuf};

fn main() {
    // Windows unresolved __imp_crc32 / deflate* / inflate* indicate the objects in librdkafka
    // were compiled expecting an import library (DLL import) for zlib, not a purely static
    // archive. Our earlier attempt to force `static=z` failed. Here we dynamically discover
    // the rdkafka-sys vcpkg-installed lib directory that already contains `zlib.lib` and
    // emit link directives for it.

    if cfg!(target_os = "windows") {
        if let Some(lib_dir) = find_rdkafka_vcpkg_zlib_dir() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            // Link the import library (no `static=` prefix) so MSVC resolves __imp_* symbols.
            println!("cargo:rustc-link-lib=zlib");
        } else {
            // Fallback: still attempt static z (may not satisfy __imp_ symbols but keeps prior behavior).
            println!("cargo:warning=Could not locate rdkafka-sys vcpkg zlib directory; falling back to static=z");
            println!("cargo:rustc-link-lib=static=z");
        }
    } else {
        // On non-Windows platforms, rely on dependent crates (e.g., libz-sys or rdkafka-sys)
        // to declare any zlib linkage as needed. Do not force static z here.
    }
}

fn find_rdkafka_vcpkg_zlib_dir() -> Option<PathBuf> {
    // Prefer explicit CARGO_TARGET_DIR if set (workspace appears to use one like C:\RustBuild\target)
    let target_root = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            // Fall back to <workspace>/target (crate is at services/auth-service)
            env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).map(|manifest_dir| {
                manifest_dir.join("..").join("..").join("target")
            })
        })?;

    let build_dir = target_root.join("debug").join("build");
    if !build_dir.is_dir() { return None; }

    // Find newest rdkafka-sys-* directory (hash changes per rebuild).
    let mut candidates: Vec<_> = fs::read_dir(&build_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("rdkafka-sys-"))
        .collect();
    if candidates.is_empty() { return None; }
    candidates.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    candidates.reverse(); // newest first

    for entry in candidates {        
        let p = entry.path()
            .join("out")
            .join("build")
            .join("vcpkg_installed")
            .join("x64-windows")
            .join("lib");
        if p.join("zlib.lib").is_file() {
            return Some(p);
        }
        // Debug variant directory may host zlibd.lib
        let debug_p = entry.path()
            .join("out")
            .join("build")
            .join("vcpkg_installed")
            .join("x64-windows")
            .join("debug")
            .join("lib");
        if debug_p.join("zlibd.lib").is_file() {
            return Some(debug_p);
        }
    }
    None
}
