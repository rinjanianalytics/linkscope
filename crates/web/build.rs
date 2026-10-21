// SPDX-License-Identifier: GPL-2.0-or-later
// Re-embed the UI whenever its build output changes.
fn main() {
    println!("cargo:rerun-if-changed=ui/dist");
    if let Ok(entries) = std::fs::read_dir("ui/dist/assets") {
        for e in entries.flatten() {
            println!("cargo:rerun-if-changed={}", e.path().display());
        }
    }
    println!("cargo:rerun-if-changed=ui/dist/index.html");
    // The manual (crate::docs) is embedded too.
    println!("cargo:rerun-if-changed=../../docs/manual");
    for lang in ["en", "id"] {
        if let Ok(entries) = std::fs::read_dir(format!("../../docs/manual/{lang}")) {
            for e in entries.flatten() {
                println!("cargo:rerun-if-changed={}", e.path().display());
            }
        }
    }
}
