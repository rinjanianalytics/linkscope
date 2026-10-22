# gNMI protocol files

`gnmi.proto` and `gnmi_ext.proto` are copied unchanged from
[openconfig/gnmi](https://github.com/openconfig/gnmi) at tag `v0.14.1`, under the Apache License 2.0
(`LICENSE` here). They sit under `github.com/openconfig/gnmi/proto/` because `gnmi.proto` imports
`gnmi_ext.proto` by that path.

`build.rs` compiles them in pure Rust (protox), so building LinkScope needs no `protoc`.
To update them, copy both files from a newer tag and change the tag above.
