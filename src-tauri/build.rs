fn main() {
    // whisper.cpp's Metal backend uses `@available` checks, which need clang's runtime
    // (`___isPlatformVersionAtLeast`). Rust links with -nodefaultlibs, so add it explicitly.
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("clang").arg("--print-resource-dir").output() {
        let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
        println!("cargo:rustc-link-search=native={dir}/lib/darwin");
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    }
    tauri_build::build()
}
