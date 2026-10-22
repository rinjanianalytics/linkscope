// SPDX-License-Identifier: GPL-2.0-or-later
//! LinkScope: a memory-safe, drop-in reimplementation of MRTG.

// Index loops over MRTG's two directions (in/out) read closer to the original.
#![allow(
    clippy::needless_range_loop,
    clippy::type_complexity,
    clippy::large_enum_variant
)]

pub mod alerts;
pub mod cfgmaker;
pub mod confcache;
pub mod cycle;
pub mod daemon;
pub mod filter;
pub mod fsutil;
pub mod gnmi;
pub mod gnmi_cfgmaker;
mod iftype_table;
mod oid_table;
pub mod oids;
pub mod poll;
pub mod snmp;
pub mod thresh;
