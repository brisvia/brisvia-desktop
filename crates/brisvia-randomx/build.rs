// Link against the SAME librandomx v1.2.2 used by the node (guarantees the same hash).
// The path is passed via RANDOMX_LIB_DIR; defaults to the Linux build on the VPS.
fn main() {
    let lib_dir = std::env::var("RANDOMX_LIB_DIR")
        .unwrap_or_else(|_| "/root/randomx-lib/build".to_string());
    println!("cargo:rustc-link-search=native={}", lib_dir);
    println!("cargo:rustc-link-lib=static=randomx");

    // RandomX is written in C++, so we must link the platform's C++ standard library.
    // Read the TARGET config (CARGO_CFG_*), which is correct even when cross-compiling.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "macos" {
        // macOS ships libc++ (not libstdc++).
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if target_os == "linux" || target_env == "gnu" {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
    if target_os == "windows" && target_env == "gnu" {
        println!("cargo:rustc-link-lib=dylib=winpthread");
    }
    // Windows: RandomX uses the privileges API (large pages) -> Advapi32.
    if target_os == "windows" {
        println!("cargo:rustc-link-lib=dylib=advapi32");
    }
}
