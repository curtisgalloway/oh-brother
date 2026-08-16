// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Compile the Swift IOBluetooth shim (swift/ptbt.swift) into a static
//! library on macOS. macOS ships the Swift runtime, so the produced
//! binary has no extra user-visible dependency.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let obj = out_dir.join("ptbt.o");
    let lib = out_dir.join("libptbt.a");

    let status = Command::new("swiftc")
        .args([
            "-parse-as-library",
            "-O",
            "-emit-object",
            "swift/ptbt.swift",
            "-o",
        ])
        .arg(&obj)
        .status()
        .expect("swiftc not found — install the Xcode command-line tools");
    assert!(status.success(), "swiftc failed");

    let status = Command::new("ar")
        .arg("crs")
        .arg(&lib)
        .arg(&obj)
        .status()
        .expect("ar not found");
    assert!(status.success(), "ar failed");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=ptbt");
    println!("cargo:rustc-link-lib=framework=IOBluetooth");
    println!("cargo:rustc-link-lib=framework=Foundation");
    // The Swift runtime; the .o's autolink load commands name the
    // individual libswift* dylibs, ld just needs the search path.
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rerun-if-changed=swift/ptbt.swift");
}
