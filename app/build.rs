fn main() {
    // RISE_ENV is read through option_env! at compile time, so cargo must be
    // told to rebuild when it changes. Without this, switching environments
    // silently reuses the previous build.
    println!("cargo:rerun-if-env-changed=RISE_ENV");
}
