use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let root = workspace_root();

    if std::env::var("CARGO_FEATURE_FFMPEG").is_ok() {
        point_rsmpeg_at_the_lgpl_prefix(&root);
    }

    if std::env::var("CARGO_FEATURE_RLOTTIE").is_ok() {
        compile_rlottie(&root);
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/rise-media has a workspace root above it")
        .to_path_buf()
}

// Point rsmpeg at the LGPL prefix built by scripts/build-ffmpeg-lgpl.sh.
//
// Without this, rsmpeg finds the system FFmpeg — which on macOS is Homebrew's,
// configured --enable-gpl with libx264 and libx265. Linking that relicenses the
// product, so finding it by accident is the failure mode worth engineering out.
fn point_rsmpeg_at_the_lgpl_prefix(root: &Path) {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let arch = if arch == "aarch64" {
        "arm64".to_string()
    } else {
        arch
    };

    let prefix = root.join("vendor/ffmpeg").join(format!("{os}-{arch}"));
    let pkgconfig = prefix.join("lib/pkgconfig");

    if !pkgconfig.exists() {
        panic!(
            "the `ffmpeg` feature needs an LGPL FFmpeg at {}.\nRun: ./scripts/build-ffmpeg-lgpl.sh",
            prefix.display()
        );
    }

    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    unsafe {
        std::env::set_var("FFMPEG_PKG_CONFIG_PATH", &pkgconfig);
        std::env::set_var("PKG_CONFIG_PATH", &pkgconfig);
    }
    println!("cargo:rerun-if-changed={}", pkgconfig.display());
}

const RLOTTIE_VERSION: &str = "0.2";

// Compile the vendored rlottie into the crate, the way rusqlite bundles SQLite.
//
// The alternative is asking three operating systems for a system librlottie
// that none of them ships, or requiring cmake and ninja on every developer
// machine and CI runner to build a shared object we would then have to sign and
// relocate inside a .app. Neither is worth it for 35 translation units.
fn compile_rlottie(root: &Path) {
    let src = root
        .join("vendor/rlottie")
        .join(format!("rlottie-{RLOTTIE_VERSION}"));

    if !src.join("inc/rlottie_capi.h").exists() {
        panic!(
            "the `rlottie` feature needs the vendored source at {}.\nRun: ./scripts/vendor-rlottie.sh",
            src.display()
        );
    }

    // Upstream generates this from cmake/config.h.in. LOTTIE_IMAGE_MODULE is
    // deliberately absent: with it, rlottie dlopens a separate
    // librlottie-image-loader at runtime, which means shipping and signing a
    // second binary. Without it, stb_image.cpp is linked in and called
    // directly. The module is not optional in effect — a sticker with an
    // embedded raster asset renders blank without an image loader.
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(
        out.join("config.h"),
        concat!(
            "#define LOTTIE_THREAD_SUPPORT\n",
            "#define LOTTIE_CACHE_SUPPORT\n",
            "#define LOTTIE_IMAGE_MODULE_PLUGIN \"\"\n",
        ),
    )
    .expect("config.h is written into OUT_DIR");

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let msvc = target_env == "msvc";

    // The pixman NEON routines that vdrawhelper_neon.cpp calls are 32-bit ARM
    // assembly, and upstream assembles them only for that target. On aarch64
    // the compiler still predefines __ARM_NEON__, so compiling the NEON helper
    // would reference symbols nothing provides — and the same macro suppresses
    // the portable memfill32 in vdrawhelper.cpp, so the link fails twice over.
    // Undefining it selects the portable path consistently.
    let arm32 = arch == "arm";
    let neon_asm = arm32;

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .include(&out)
        .include(src.join("inc"))
        .include(src.join("src/lottie"))
        .include(src.join("src/vector"))
        .include(src.join("src/vector/freetype"))
        .include(src.join("src/vector/pixman"))
        .include(src.join("src/vector/stb"))
        .include(src.join("src/binding/c"))
        .define("RLOTTIE_BUILD", None)
        .warnings(false);

    if msvc {
        build.flag("/std:c++14").flag("/EHsc-").flag("/GR-");
    } else {
        build
            .flag("-std=c++14")
            .flag("-fno-exceptions")
            .flag("-fno-rtti")
            .flag("-fvisibility=hidden");
        if !neon_asm {
            build.flag("-U__ARM_NEON__");
        }
    }

    for file in rlottie_sources(&src, neon_asm) {
        build.file(file);
    }

    build.compile("rlottie");

    if neon_asm {
        cc::Build::new()
            .file(src.join("src/vector/pixman/pixman-arm-neon-asm.S"))
            .flag("-x")
            .flag("assembler-with-cpp")
            .compile("rlottie_pixman_neon");
    }

    // Not linked through cc because the C++ runtime is not one of cc's outputs:
    // rlottie uses std::string, std::vector and std::thread throughout.
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if !msvc {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }

    println!("cargo:rerun-if-changed={}", src.join("src").display());
    println!("cargo:rerun-if-changed={}", src.join("inc").display());
}

fn rlottie_sources(src: &Path, neon: bool) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();

    for directory in [
        "src/lottie",
        "src/vector",
        "src/vector/freetype",
        "src/vector/stb",
        "src/binding/c",
    ] {
        let entries = std::fs::read_dir(src.join(directory))
            .unwrap_or_else(|error| panic!("reading {directory}: {error}"));

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("cpp") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if stem == "vdrawhelper_neon" && !neon {
                continue;
            }
            files.push(path);
        }
    }

    // read_dir order is filesystem order, which differs between machines and
    // makes the compiled object list — and therefore ccache and sccache keys —
    // nondeterministic.
    files.sort();
    files
}
