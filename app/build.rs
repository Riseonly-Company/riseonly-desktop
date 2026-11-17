#[path = "../scripts/windows_resource.rs"]
mod windows_resource;

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=RISE_ENV");
    println!("cargo:rerun-if-env-changed=RISE_BUILD_NUMBER");

    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => embed_windows_resource(),
        Ok("linux") => point_loader_at_bundled_ffmpeg(),
        Ok("macos") => keep_webrtc_objc_categories(),
        _ => {}
    }
}

fn keep_webrtc_objc_categories() {
    println!("cargo:rustc-link-arg-bins=-ObjC");
}

fn point_loader_at_bundled_ffmpeg() {
    println!("cargo:rustc-link-arg-bin=riseonly=-Wl,--enable-new-dtags");
    println!("cargo:rustc-link-arg-bin=riseonly=-Wl,-rpath,$ORIGIN/../lib/riseonly");

    if env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_none() {
        return;
    }
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64".to_string(),
        Ok(other) => other.to_string(),
        Err(_) => return,
    };
    let workspace_root = manifest_dir
        .parent()
        .expect("app has the workspace root above it");
    let prefix_lib = workspace_root.join(format!("vendor/ffmpeg/linux-{arch}/lib"));
    println!(
        "cargo:rustc-link-arg-bin=riseonly=-Wl,-rpath,{}",
        prefix_lib.display()
    );
}

fn embed_windows_resource() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let template_path = manifest_dir.join("resources/windows/riseonly.rc.in");
    let icon_path = manifest_dir.join("resources/windows/app.ico");
    let values_path = manifest_dir.join("../scripts/windows_resource.rs");
    for input in [&template_path, &icon_path, &values_path] {
        println!("cargo:rerun-if-changed={}", input.display());
    }

    let package = |name: &str| env::var(name).unwrap_or_else(|_| panic!("cargo sets {name}"));
    let values = windows_resource::ResourceValues::resolve(
        env::var("RISE_ENV").ok().as_deref(),
        env::var("RISE_BUILD_NUMBER").ok().as_deref(),
        env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some(),
        [
            &package("CARGO_PKG_VERSION"),
            &package("CARGO_PKG_VERSION_MAJOR"),
            &package("CARGO_PKG_VERSION_MINOR"),
            &package("CARGO_PKG_VERSION_PATCH"),
        ],
    )
    .expect("every canon environment has a display name in scripts/windows_resource.rs");

    let template = std::fs::read_to_string(&template_path)
        .unwrap_or_else(|error| panic!("{}: {error}", template_path.display()));
    let resource = values
        .render(&template, &icon_path.to_string_lossy())
        .unwrap_or_else(|error| panic!("{error}"));
    let resource_path = out_dir.join("riseonly.rc");
    std::fs::write(&resource_path, resource)
        .unwrap_or_else(|error| panic!("{}: {error}", resource_path.display()));

    embed_resource::compile_for(&resource_path, ["riseonly"], embed_resource::NONE)
        .manifest_required()
        .unwrap_or_else(|error| panic!("{error}"));
}
