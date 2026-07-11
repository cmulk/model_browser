use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let vendor_dir = Path::new("frontend/vendor");
    let three_path = vendor_dir.join("three.module.js");
    let orbit_path = vendor_dir.join("OrbitControls.js");

    println!("cargo:rerun-if-changed=frontend/vendor/three.module.js");
    println!("cargo:rerun-if-changed=frontend/vendor/OrbitControls.js");

    if !vendor_dir.exists() {
        fs::create_dir_all(vendor_dir).expect("Failed to create frontend/vendor directory");
    }

    if !three_path.exists() {
        println!("Downloading three.module.js...");
        let status = Command::new("curl")
            .args([
                "-sL",
                "https://cdn.jsdelivr.net/npm/three@0.166.1/build/three.module.min.js",
                "-o",
                three_path.to_str().unwrap(),
            ])
            .status()
            .expect("Failed to execute curl to download three.module.js");
        if !status.success() {
            panic!("Failed to download three.module.js via curl");
        }
    }

    if !orbit_path.exists() {
        println!("Downloading OrbitControls.js...");
        let status = Command::new("curl")
            .args([
                "-sL",
                "https://cdn.jsdelivr.net/npm/three@0.166.1/examples/jsm/controls/OrbitControls.js",
                "-o",
                orbit_path.to_str().unwrap(),
            ])
            .status()
            .expect("Failed to execute curl to download OrbitControls.js");
        if !status.success() {
            panic!("Failed to download OrbitControls.js via curl");
        }
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("Failed to embed Windows icon resource");
    }
}
