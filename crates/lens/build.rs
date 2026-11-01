// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! Embeds every XML file of the Lensfun database (`lensfun/db`), as it is.

use std::fmt::Write;
use std::path::PathBuf;

fn main() {
    let directory = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"))
        .join("lensfun")
        .join("db");
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)
        .expect("lensfun/db")
        .map(|entry| entry.expect("lensfun/db entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "xml"))
        .collect();
    files.sort();
    let mut source = String::from(
        "/// The database's XML files, by name.\npub static FILES: &[(&str, &str)] = &[\n",
    );
    for path in files {
        let name = path.file_name().expect("file").to_string_lossy();
        writeln!(
            source,
            "    ({name:?}, include_str!({:?})),",
            path.display().to_string()
        )
        .expect("String");
    }
    source.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it")).join("database.rs");
    std::fs::write(out, source).expect("OUT_DIR");
}
