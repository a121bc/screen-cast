fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target != "windows" {
        panic!("the sender binary is only supported on Windows targets");
    }
}
