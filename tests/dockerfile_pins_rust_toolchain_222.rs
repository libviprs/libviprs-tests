//! The gate image's Rust toolchain must be pinned, and pinned to the core's MSRV (#222).
//!
//! `Dockerfile` used to say `FROM rust:latest`. When `latest` moved to 1.99 it
//! deprecated `Atomic::fetch_update`, the core denies deprecations
//! (`[lints.rust] deprecated = "deny"`), and the pre-push gate stopped compiling
//! the core at all. Nothing about the pushed branch had changed, so the failure
//! read as the branch's fault. A floating tag makes the gate's result depend on
//! the day it ran, which is what a gate must not do.
//!
//! This reads the builder stage's base image, resolves any global `ARG`
//! default it uses, and requires a concrete version tag that matches the core's
//! `rust-version`, so the two have to move together. It is a text check on a
//! checked-in file, needs no docker, and runs under the default `cargo test`.

use std::path::PathBuf;

fn read(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("failed to read {}: {e}", p.display()))
}

/// The `[package]` `rust-version` from the core manifest, e.g. `1.97`.
fn core_msrv() -> String {
    read("../libviprs/Cargo.toml")
        .lines()
        .find_map(|l| {
            let v = l.trim().strip_prefix("rust-version")?;
            Some(
                v.trim_start()
                    .strip_prefix('=')?
                    .trim()
                    .trim_matches('"')
                    .to_owned(),
            )
        })
        .expect("core Cargo.toml has no `rust-version`")
}

/// Global `ARG NAME=default` values (those before the first `FROM`).
fn global_args(dockerfile: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in dockerfile.lines().map(str::trim) {
        if line.starts_with("FROM ") {
            break;
        }
        if let Some((k, v)) = line.strip_prefix("ARG ").and_then(|a| a.split_once('=')) {
            out.push((k.trim().to_owned(), v.trim().trim_matches('"').to_owned()));
        }
    }
    out
}

/// Every `FROM rust:<tag>` image reference with `${ARG}` substituted.
fn rust_images(dockerfile: &str) -> Vec<String> {
    let args = global_args(dockerfile);
    dockerfile
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("FROM "))
        .map(|rest| {
            let mut image = rest.split_whitespace().next().unwrap_or("").to_owned();
            for (k, v) in &args {
                image = image
                    .replace(&format!("${{{k}}}"), v)
                    .replace(&format!("${k}"), v);
            }
            image
        })
        .filter(|i| i.starts_with("rust:") || i == "rust")
        .collect()
}

/// Everything wrong with the builder's Rust image in `dockerfile`, measured
/// against core MSRV `msrv`. Empty means the pin is fine. Split out from the
/// test so the controls below run the same check over Dockerfiles that must fail.
fn dockerfile_problems(dockerfile: &str, msrv: &str) -> Vec<String> {
    let images = rust_images(dockerfile);
    if images.is_empty() {
        return vec!["Dockerfile has no `FROM rust:...` stage".to_owned()];
    }
    let mut problems = Vec::new();
    for image in &images {
        let tag = image.strip_prefix("rust:").unwrap_or("");
        if tag.is_empty() || tag == "latest" || !tag.starts_with(|c: char| c.is_ascii_digit()) {
            problems.push(format!(
                "`FROM {image}` floats: pin a version tag so the gate's toolchain cannot change under it (#222)"
            ));
        } else if !(tag == msrv
            || tag.starts_with(&format!("{msrv}."))
            || tag.starts_with(&format!("{msrv}-")))
        {
            problems.push(format!(
                "`FROM {image}` is not the core's MSRV {msrv}; move the pin together with `rust-version` (#222)"
            ));
        }
    }
    problems
}

#[test]
fn dockerfile_rust_image_is_pinned_to_the_core_msrv_222() {
    let problems = dockerfile_problems(&read("Dockerfile"), &core_msrv());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

// Controls: the same check over Dockerfiles it has to reject (or accept). A
// guard that never fails proves nothing, and the first two shapes below are
// ones it used to wave through.

/// A `--platform` flag sits between `FROM` and the image. Taking the first
/// token after `FROM` as the image skipped this stage entirely. A good builder
/// stage sits next to it, so the guard can't pass this by noticing that no
/// stage was found at all.
#[test]
fn control_a_platform_flag_does_not_hide_a_floating_tag_222() {
    let df = "ARG RUST_VERSION=1.97\n\
              FROM rust:${RUST_VERSION}-bookworm AS builder\n\
              FROM --platform=$BUILDPLATFORM rust:latest AS tools\n";
    assert!(
        !dockerfile_problems(df, "1.97").is_empty(),
        "`{df}` passed the guard"
    );
}

/// Dockerfile instructions are case-insensitive, so `from` is a stage too,
/// again next to a good builder stage.
#[test]
fn control_a_lowercase_from_is_still_a_stage_222() {
    let df = "ARG RUST_VERSION=1.97\n\
              FROM rust:${RUST_VERSION}-bookworm AS builder\n\
              from rust:latest as tools\n";
    assert!(
        !dockerfile_problems(df, "1.97").is_empty(),
        "`{df}` passed the guard"
    );
}

/// The tag comes from the one `RUST_VERSION` arg, not a literal typed into the
/// `FROM` line, so there's a single place to move it.
#[test]
fn control_a_literal_tag_that_skips_the_arg_is_rejected_222() {
    let df = "ARG RUST_VERSION=1.97\nFROM rust:1.97-bookworm AS builder\n";
    assert!(
        !dockerfile_problems(df, "1.97").is_empty(),
        "`FROM rust:1.97-bookworm` passed without using ${{RUST_VERSION}}"
    );
}

/// A toolchain below the core's MSRV can't build the core.
#[test]
fn control_a_toolchain_below_the_msrv_is_rejected_222() {
    let df = "ARG RUST_VERSION=1.96\nFROM rust:${RUST_VERSION}-bookworm AS builder\n";
    assert!(
        !dockerfile_problems(df, "1.97").is_empty(),
        "1.96 passed against MSRV 1.97"
    );
}

/// A concrete toolchain newer than the MSRV is fine; the guard asks for a
/// concrete pin at or above it, not for equality.
#[test]
fn control_a_concrete_newer_toolchain_is_accepted_222() {
    let df = "ARG RUST_VERSION=1.99.1\nFROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-bookworm AS builder\n";
    let problems = dockerfile_problems(df, "1.97");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
