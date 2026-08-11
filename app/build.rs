fn main() {
    // RISE_ENV is read via option_env! at compile time, so a change must force a rebuild.
    println!("cargo:rerun-if-env-changed=RISE_ENV");
}
