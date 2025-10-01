// NOTE: Single build script variant retained (below) that copies runtime DLLs and links zlib.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=PROFILE");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let Some(out_dir_os) = env::var_os("OUT_DIR") else {
        return;
    };
    let out_dir = PathBuf::from(out_dir_os);

    let Some(build_root) = build_root_from_out_dir(&out_dir) else {
        return;
    };
    let Some(target_profile_dir) = target_profile_dir(&out_dir) else {
        return;
    };

    // librdkafka vendors its Windows dependencies via vcpkg; reuse that zlib build.
    let Some(vcpkg_root) = find_rdkafka_vcpkg_root(&build_root) else {
        return;
    };

    let release_lib_dir = vcpkg_root.join("lib");
    let debug_lib_dir = vcpkg_root.join("debug").join("lib");

    if release_lib_dir.exists() {
        println!(
            "cargo:rustc-link-search=native={}",
            release_lib_dir.display()
        );
    }
    if debug_lib_dir.exists() {
        println!("cargo:rustc-link-search=native={}", debug_lib_dir.display());
    }

    let profile = env::var("PROFILE").unwrap_or_default();
    let is_debug = profile.eq_ignore_ascii_case("debug")
        || env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some();

    if is_debug && debug_lib_dir.join("zlibd.lib").exists() {
        println!("cargo:rustc-link-lib=dylib=zlibd");
    } else {
        println!("cargo:rustc-link-lib=dylib=zlib");
    }

    copy_runtime_dlls(&vcpkg_root.join("bin"), &target_profile_dir);
    if is_debug {
        copy_runtime_dlls(&vcpkg_root.join("debug").join("bin"), &target_profile_dir);
    }
}

fn build_root_from_out_dir(out_dir: &Path) -> Option<PathBuf> {
    let build_dir = out_dir.parent()?; // order-service-<hash>
    build_dir.parent().map(Path::to_path_buf)
}

fn target_profile_dir(out_dir: &Path) -> Option<PathBuf> {
    out_dir.ancestors().nth(3).map(Path::to_path_buf)
}

fn find_rdkafka_vcpkg_root(build_root: &Path) -> Option<PathBuf> {
    let mut newest: Option<(PathBuf, SystemTime)> = None;

    let entries = fs::read_dir(build_root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name() else {
            continue;
        };
        if !name.to_string_lossy().starts_with("rdkafka-sys-") {
            continue;
        }

        let candidate = path
            .join("out")
            .join("build")
            .join("vcpkg_installed")
            .join("x64-windows");
        if !candidate.exists() {
            continue;
        }

        let metadata = entry.metadata().ok()?;
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        match &mut newest {
            Some((best_path, best_time)) => {
                if modified > *best_time {
                    *best_path = candidate.clone();
                    *best_time = modified;
                }
            }
            None => newest = Some((candidate, modified)),
        }
    }

    newest.map(|(path, _)| path)
}

fn copy_runtime_dlls(source_dir: &Path, destination_dir: &Path) {
    if !source_dir.exists() {
        return;
    }

    if let Err(err) = fs::create_dir_all(destination_dir) {
        println!(
            "cargo:warning=order-service: unable to prepare {}: {}",
            destination_dir.display(),
            err
        );
        return;
    }

    let entries = match fs::read_dir(source_dir) {
        Ok(entries) => entries,
        Err(err) => {
            println!(
                "cargo:warning=order-service: unable to read {}: {}",
                source_dir.display(),
                err
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !ext.eq_ignore_ascii_case("dll") {
            continue;
        }

        let file_name = match path.file_name() {
            Some(name) => name,
            None => continue,
        };

        let destination = destination_dir.join(file_name);
        let should_copy = match (fs::metadata(&path), fs::metadata(&destination)) {
            (Ok(src_meta), Ok(dest_meta)) => {
                let src_time = src_meta.modified().unwrap_or(UNIX_EPOCH);
                let dest_time = dest_meta.modified().unwrap_or(UNIX_EPOCH);
                src_time > dest_time
            }
            _ => true,
        };

        if should_copy {
            if let Err(err) = fs::copy(&path, &destination) {
                println!(
                    "cargo:warning=order-service: failed to copy {} to {}: {}",
                    path.display(),
                    destination.display(),
                    err
                );
            }
        }
    }
}
