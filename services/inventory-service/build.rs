use std::{env, fs, path::PathBuf};

fn main() {
    if cfg!(target_os = "windows") {
        if let Some(dir) = find_rdkafka_vcpkg_zlib_dir() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=zlib");
            println!("cargo:warning=inventory-service linking zlib from {}", dir.display());
        } else {
            println!("cargo:warning=inventory-service could not locate zlib import library; falling back to static=z");
            println!("cargo:rustc-link-lib=static=z");
        }
    } else {
        println!("cargo:rustc-link-lib=static=z");
    }
}

fn find_rdkafka_vcpkg_zlib_dir() -> Option<PathBuf> {
    let target_root = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).map(|m| m.join("..").join("..").join("target")))?;
    let build_dir = target_root.join("debug").join("build");
    if !build_dir.is_dir() { return None; }
    let mut candidates: Vec<_> = fs::read_dir(&build_dir).ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("rdkafka-sys-"))
        .collect();
    if candidates.is_empty() { return None; }
    candidates.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    candidates.reverse();
    for entry in candidates {
        let base = entry.path().join("out").join("build").join("vcpkg_installed").join("x64-windows");
        let rel = base.join("lib");
        if rel.join("zlib.lib").is_file() { return Some(rel); }
        let debug_rel = base.join("debug").join("lib");
        if debug_rel.join("zlibd.lib").is_file() { return Some(debug_rel); }
    }
    None
}
