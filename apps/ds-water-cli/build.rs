// The native Water addon's build identity (ds.engine-build/v1), as Core and
// the release producer admit it: the exact source, target, profile, compiler
// and lockfile, and one digest over them. A development build says so.
use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-env-changed=DS_WATER_SOURCE_SHA");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets the manifest dir");
    let lock_path = std::path::Path::new(&manifest_dir).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());

    let source = std::env::var("DS_WATER_SOURCE_SHA").unwrap_or_else(|_| "development".into());
    let profile = std::env::var("PROFILE").expect("cargo sets the profile");
    if profile == "release" || source != "development" {
        assert!(
            source.len() == 40
                && source
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "DS_WATER_SOURCE_SHA must be the full 40-digit source SHA of a release build"
        );
    }
    let target = std::env::var("TARGET").expect("cargo sets the target");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let rustc = std::process::Command::new(rustc)
        .arg("-V")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| text.trim().to_owned())
        .unwrap_or_default();
    let lock = std::fs::read(&lock_path).expect("ds-water is a locked workspace");
    let lock_sha = format!("{:x}", Sha256::digest(&lock));
    let manifest = format!("{source}\n{target}\n{profile}\n{rustc}\n{lock_sha}\n");
    let manifest_sha = format!("{:x}", Sha256::digest(manifest.as_bytes()));

    for (name, value) in [
        ("DS_WATER_SOURCE_SHA", source.as_str()),
        ("DS_WATER_TARGET", target.as_str()),
        ("DS_WATER_PROFILE", profile.as_str()),
        ("DS_WATER_RUSTC", rustc.as_str()),
        ("DS_WATER_CARGO_LOCK_SHA256", lock_sha.as_str()),
        ("DS_WATER_BUILD_MANIFEST_SHA256", manifest_sha.as_str()),
    ] {
        println!("cargo:rustc-env={name}={value}");
    }
}
