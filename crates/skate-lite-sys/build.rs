// Builds the vendored clean-room engine (vendor/src/*.cpp, C++17).
fn main() {
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").include("vendor/include").warnings(false);
    let mut files: Vec<_> = std::fs::read_dir("vendor/src")
        .expect("vendor/src missing: run scripts/cleanroom/sync-lite.sh")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "cpp"))
        .collect();
    files.sort();
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        build.file(f);
    }
    println!("cargo:rerun-if-changed=vendor/include/sk8.h");
    println!("cargo:rerun-if-changed=vendor/src");
    // libc++ on Android comes from the NDK's shared runtime (bevy's `android_shared_stdcxx`).
    build.cpp_link_stdlib(None);
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("apple") {
        println!("cargo:rustc-link-lib=c++");
    } else if target.contains("android") {
        println!("cargo:rustc-link-lib=c++_shared");
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }
    build.compile("sk8");
}
