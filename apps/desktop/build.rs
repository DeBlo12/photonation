// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! On Windows, embeds the app icon and version information in the
//! executable, so Explorer, the taskbar and Start show the Photonation icon.

fn main() {
    println!("cargo::rerun-if-changed=../../assets/icon/photonation.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("../../assets/icon/photonation.ico")
        .set("ProductName", "Photonation")
        .set("FileDescription", "Photonation")
        .set(
            "LegalCopyright",
            "Copyright (C) 2026 The Photonation contributors",
        );
    if let Err(error) = resource.compile() {
        panic!("embedding the Windows icon failed: {error}");
    }
}
