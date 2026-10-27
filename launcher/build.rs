// The launcher uses the game's version: one version scheme for the fork, one release tag.
fn main() {
    println!("cargo:rerun-if-changed=../VERSION");
    let v = std::fs::read_to_string("../VERSION").unwrap_or_else(|_| "0.4.0".to_string());
    println!("cargo:rustc-env=RAC_RS_VERSION={}", v.trim());
}
