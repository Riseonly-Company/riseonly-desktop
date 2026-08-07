use std::path::{Path, PathBuf};
use std::process::Command;

/// Matches `MacOsVersion::VIBRANCY` and `LSMinimumSystemVersion` in the bundle.
/// The Swift is compiled against the product's floor, not the build machine's
/// SDK, so `#available(macOS 26.0, *)` inside it stays a real runtime check
/// instead of being folded away.
const DEPLOYMENT_TARGET: &str = "13.0";

const MODULE: &str = "riseglass";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // Not cfg!(target_os): a build script runs on the host, and the thing being
    // decided is what the *target* needs.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/macos/glass/RiseGlass.swift");
    println!("cargo:rerun-if-changed={}", source.display());

    compile_rise_glass(&source, &swift());
}

/// Where the Swift compiler is, and which SDK it must be told about.
///
/// `/usr/bin/swiftc` is a shim that asks `xcrun` for both; the compiler inside
/// the toolchain is not, and invoking it with an explicit `-target` and no
/// `-sdk` fails with "unable to load standard library". Resolving the pair
/// together is what keeps that from depending on which one happens to be found.
struct Swift {
    compiler: PathBuf,
    sdk: Option<PathBuf>,
}

fn compile_rise_glass(source: &Path, swift: &Swift) {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let archive = out.join(format!("lib{MODULE}.a"));

    let mut command = Command::new(&swift.compiler);
    command
        // Pinned rather than left at the toolchain default, so that a newer
        // Swift shipping a new default language mode cannot change what this
        // file means. Moving it is deliberate work, not a side effect of
        // installing Xcode.
        .args(["-swift-version", "5"])
        .arg("-parse-as-library")
        .args(["-module-name", MODULE])
        .args([
            "-target",
            &format!("{}-apple-macosx{DEPLOYMENT_TARGET}", arch()),
        ])
        .arg("-emit-library")
        .arg("-static")
        .arg(optimisation())
        .arg("-o")
        .arg(&archive)
        .arg(source);

    if let Some(sdk) = &swift.sdk {
        command.arg("-sdk").arg(sdk);
    }

    let status = command
        .status()
        .unwrap_or_else(|error| panic!("running {}: {error}", swift.compiler.display()));

    assert!(
        status.success(),
        "swiftc failed to build {}",
        source.display()
    );

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static={MODULE}");

    // The Swift objects carry LC_LINKER_OPTION autolink directives for every
    // stdlib and framework they touch, so nothing here has to enumerate them.
    // What the linker still needs is somewhere to find them: the ABI-stable
    // runtime that ships with macOS itself.
    println!("cargo:rustc-link-search=native=/usr/lib/swift");

    // The compatibility shims a newer compiler links against an older runtime
    // are static archives in the toolchain rather than in /usr/lib/swift.
    if let Some(toolchain) = toolchain_library_path() {
        println!("cargo:rustc-link-search=native={}", toolchain.display());
    }
}

fn arch() -> String {
    match std::env::var("CARGO_CFG_TARGET_ARCH")
        .unwrap_or_default()
        .as_str()
    {
        "aarch64" => "arm64".to_owned(),
        other => other.to_owned(),
    }
}

fn optimisation() -> &'static str {
    match std::env::var("PROFILE").as_deref() {
        Ok("release") => "-O",
        _ => "-Onone",
    }
}

/// Loud on purpose. A macOS build with no Swift compiler cannot produce a
/// working app — the glass regions would be missing with no diagnostic anywhere
/// — so this is a hard failure carrying the command that fixes it, never a
/// silent skip that ships a blank rail.
fn swift() -> Swift {
    if let Some(compiler) = xcrun_path(&["--find", "swiftc"]) {
        return Swift {
            compiler,
            sdk: xcrun_path(&["--show-sdk-path"]),
        };
    }

    if let Ok(status) = Command::new("swiftc").arg("--version").status()
        && status.success()
    {
        return Swift {
            compiler: PathBuf::from("swiftc"),
            sdk: None,
        };
    }

    panic!(
        "rise-platform needs the Swift compiler to build the RiseGlass bridge on macOS.\n\
         Install the Xcode command line tools, then retry:\n\
         \n\
         \x20   xcode-select --install\n\
         \n\
         If Xcode is already installed, point xcode-select at it:\n\
         \n\
         \x20   sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer\n"
    );
}

fn xcrun_path(args: &[&str]) -> Option<PathBuf> {
    let output = Command::new("xcrun").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    path.exists().then_some(path)
}

fn toolchain_library_path() -> Option<PathBuf> {
    let path = xcrun_path(&["--find", "swiftc"])?
        .parent()?
        .parent()?
        .join("lib/swift/macosx");

    path.is_dir().then_some(path)
}
