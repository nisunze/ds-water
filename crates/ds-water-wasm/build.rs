// A release build carries the producer's exact source and dependency-closure
// identity; a development build says so explicitly (as ds-solar-wasm does).
fn main() {
    let release = std::env::var("PROFILE").is_ok_and(|value| value == "release");
    for (name, length) in [
        ("DS_WATER_WASM_SOURCE_SHA", 40),
        ("DS_WATER_WASM_DEPENDENCY_CLOSURE_SHA256", 64),
    ] {
        println!("cargo:rerun-if-env-changed={name}");
        let value = std::env::var(name).unwrap_or_else(|_| "development".into());
        if release || value != "development" {
            assert!(
                value.len() == length
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{name} must be a full {length}-digit hexadecimal release identity"
            );
        }
        println!("cargo:rustc-env={name}={value}");
    }
}
