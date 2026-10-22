// SPDX-License-Identifier: GPL-2.0-or-later
//! The gNMI protocol (gRPC Network Management Interface), generated from openconfig/gnmi's
//! `gnmi.proto` and `gnmi_ext.proto` (Apache-2.0, see `proto/`).
//!
//! LinkScope uses the client to read interface counters; `mockagent` uses the server to answer
//! like a router in tests.

#![allow(clippy::all, clippy::pedantic, missing_docs)]

pub mod gnmi_ext {
    tonic::include_proto!("gnmi_ext");
}

pub mod gnmi {
    tonic::include_proto!("gnmi");
}

pub use gnmi::*;
