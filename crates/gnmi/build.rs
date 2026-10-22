// SPDX-License-Identifier: GPL-2.0-or-later
//! Compile the gNMI proto files in pure Rust (protox), so the build needs no `protoc`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = "proto";
    let file = "github.com/openconfig/gnmi/proto/gnmi/gnmi.proto";
    println!("cargo:rerun-if-changed={root}");
    let fds = protox::compile([file], [root])?;
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds(fds)?;
    Ok(())
}
