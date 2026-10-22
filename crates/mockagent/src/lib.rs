// SPDX-License-Identifier: GPL-2.0-or-later
//! Test devices for LinkScope.
//!
//! [`gnmi`] answers gNMI like a router with the OpenConfig model or Nokia SR Linux's own, so the
//! gNMI client is tested without a router: login, TLS, `Get` and `Subscribe` (`ONCE`), each
//! encoding, unknown interfaces, and devices that answer a leaf with its parent container.

pub mod gnmi;
