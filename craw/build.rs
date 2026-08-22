fn main() {
    let git_hash = std::env::var("CRAW_GIT_HASH").unwrap_or_else(|_| {
        std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    });
    println!("cargo:rustc-env=CRAW_GIT_HASH={}", git_hash);

    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    println!("cargo:rustc-env=CRAW_TARGET_ARCH={}", target_arch);
    println!("cargo:rustc-env=CRAW_TARGET_OS={}", target_os);
}
