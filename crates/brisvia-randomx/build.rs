// Link against the SAME librandomx v1.2.2 used by the node (guarantees the same hash).
// The path is passed via RANDOMX_LIB_DIR; defaults to the Linux build on the VPS.
fn main() {
    let lib_dir = std::env::var("RANDOMX_LIB_DIR")
        .unwrap_or_else(|_| "/root/randomx-lib/build".to_string());
    println!("cargo:rustc-link-search=native={}", lib_dir);
    println!("cargo:rustc-link-lib=static=randomx");
    // RandomX is written in C++ -> we must link libstdc++ (Linux/mingw).
    if cfg!(target_env = "gnu") || cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
    if cfg!(all(target_os = "windows", target_env = "gnu")) {
        println!("cargo:rustc-link-lib=dylib=winpthread");
    }
    // Windows: RandomX uses the privileges API (large pages) -> Advapi32.
    if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-lib=dylib=advapi32");
    }
}
