use std::path::PathBuf;
use std::{env, fs};

fn main() {
    libmpv();
    tauri_build::build()
}

/// Links libmpv and puts its DLL beside the app and the test binaries.
fn libmpv() {
    println!("cargo:rerun-if-env-changed=LIBMPV_DIR");
    let dir = env::var_os("LIBMPV_DIR").map_or_else(
        || required_var("CARGO_MANIFEST_DIR").join("binaries"),
        PathBuf::from,
    );
    let dll = dir.join("libmpv-2.dll");
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rerun-if-changed={}", dll.display());
    println!("cargo:rerun-if-changed={}", dir.join("mpv.lib").display());

    // OUT_DIR is somewhere below target/<profile>/build; the depth varies.
    let out = required_var("OUT_DIR");
    let profile = out
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "build"))
        .and_then(|build| build.parent())
        .expect("OUT_DIR lies below a build directory");
    for folder in [profile.to_path_buf(), profile.join("deps")] {
        let copied =
            fs::create_dir_all(&folder).and_then(|()| fs::copy(&dll, folder.join("libmpv-2.dll")));
        if let Err(error) = copied {
            panic!(
                "cannot copy {} to {}: {error}. Run scripts/fetch-pinned-binaries.ps1 first",
                dll.display(),
                folder.display()
            );
        }
    }
}

fn required_var(name: &str) -> PathBuf {
    PathBuf::from(env::var_os(name).unwrap_or_else(|| panic!("cargo sets {name}")))
}
