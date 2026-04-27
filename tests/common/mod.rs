//! Shared test helpers.
//!
//! Each `tests/*.rs` file is compiled as a standalone integration-test
//! binary, so anything they share has to live here under `tests/common/`
//! and be pulled in with `mod common;` at the top of the test file. The
//! `#[allow(dead_code)]` on the helpers below is intentional — not every
//! consumer uses every helper, and Cargo would otherwise emit a
//! dead-code warning per unused symbol per test binary.

#![allow(dead_code)]

pub mod fixtures;
