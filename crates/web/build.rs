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
}
