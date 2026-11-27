//! Tauri's build step, then libmpv: points the linker at the local copy and puts the DLL next to
//! the built binaries.
use std::{env, fs, path::PathBuf};

fn main() {
    tauri_build::build();

    println!("cargo:rerun-if-env-changed=LIBMPV_DIR");
    let dir = env::var_os("LIBMPV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA"))
                .join("Programs/libmpv")
        });
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!(
        "cargo:rerun-if-changed={}",
        dir.join("libmpv-2.dll").display()
    );

    // OUT_DIR is below target/<profile>/build/; the depth differs between cargo versions.
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let profile = out
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build"))
        .and_then(|p| p.parent())
        .expect("profile dir");
    for dest in [profile.to_path_buf(), profile.join("deps")] {
        fs::create_dir_all(&dest).expect("create target dir");
        fs::copy(dir.join("libmpv-2.dll"), dest.join("libmpv-2.dll")).expect("copy libmpv-2.dll");
    }
}
