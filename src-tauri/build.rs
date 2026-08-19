use std::{env, path::{Path, PathBuf}, process::Command};

fn main() {
    #[cfg(target_os = "macos")]
    build_audio_tap();
    tauri_build::build()
}

#[cfg(target_os = "macos")]
fn build_audio_tap() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let source = manifest.join("audio-helper/SystemAudioTap.swift");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let output = out_dir.join("interview-audio-tap");
    let module_cache = out_dir.join("SwiftModuleCache");
    std::fs::create_dir_all(&module_cache).expect("failed to create Swift module cache");
    let sdk = ["MacOSX15.5.sdk", "MacOSX15.4.sdk", "MacOSX15.2.sdk", "MacOSX14.5.sdk"]
        .iter()
        .map(|name| Path::new("/Library/Developer/CommandLineTools/SDKs").join(name))
        .find(|path| path.exists())
        .expect("no compatible macOS SDK found in Xcode Command Line Tools");
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => panic!("unsupported macOS target architecture"),
    };
    let target = format!("{architecture}-apple-macosx14.4");
    println!("cargo:rerun-if-changed={}", source.display());

    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "-swift-version",
            "5",
            "-parse-as-library",
            "-sdk",
        ])
        .arg(&sdk)
        .arg("-target")
        .arg(&target)
        .arg("-module-cache-path")
        .arg(&module_cache)
        .arg(&source)
        .arg("-o")
        .arg(&output)
        .status()
        .expect("failed to invoke swiftc; install Xcode Command Line Tools");
    assert!(status.success(), "failed to compile the Core Audio helper");
    println!("cargo:rustc-env=INTERVIEW_AUDIO_TAP_BIN={}", output.display());
}
