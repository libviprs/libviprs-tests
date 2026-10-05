//! Reading a `Cargo.toml` `[features]` table without a TOML dependency.
//!
//! Three guards ask the same question of three manifests (this crate's, the
//! core's and the cli's): which features does it declare, and what does each
//! one turn on. `tests/ci_feature_coverage.rs` used to carry its own line
//! parser for the first of those; it lives here now so the
//! `tests/cli_surface_coverage.rs` guard reads the other two the same way
//! instead of growing a second copy that drifts from the first.

/// One `name = [ ... ]` entry of a `[features]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feature {
    /// The feature name, as `--features` spells it.
    pub name: String,
    /// What it enables, quotes stripped (`"libviprs/jxl"` reads `libviprs/jxl`).
    pub enables: Vec<String>,
}

/// Every feature in `manifest`'s `[features]` table, in declaration order,
/// `default` included (callers drop it when they mean "capabilities").
///
/// Handles the two shapes the org's manifests use: a one-line array, and an
/// array that opens on the name line and closes on a later one (the cli's
/// `full = [ ... ]`). Comments, blank lines and trailing `#` comments are
/// skipped. Anything else that is not `name = [...]` is ignored rather than
/// guessed at.
pub fn features(manifest: &str) -> Vec<Feature> {
    let mut out: Vec<Feature> = Vec::new();
    let mut inside = false;
    // The feature whose array is still open, if any.
    let mut open: Option<Feature> = None;

    for raw in manifest.lines() {
        let line = strip_comment(raw).trim();
        if open.is_none() && line.starts_with('[') && line.ends_with(']') && !line.contains('=') {
            inside = line == "[features]";
            continue;
        }
        if !inside || line.is_empty() {
            continue;
        }

        if let Some(mut feature) = open.take() {
            let (body, closed) = match line.split_once(']') {
                Some((body, _)) => (body, true),
                None => (line, false),
            };
            feature.enables.extend(quoted(body));
            if closed {
                out.push(feature);
            } else {
                open = Some(feature);
            }
            continue;
        }

        let Some((name, rest)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let rest = rest.trim();
        let Some(rest) = rest.strip_prefix('[') else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let mut feature = Feature {
            name: name.to_string(),
            enables: Vec::new(),
        };
        match rest.split_once(']') {
            Some((body, _)) => {
                feature.enables.extend(quoted(body));
                out.push(feature);
            }
            None => {
                feature.enables.extend(quoted(rest));
                open = Some(feature);
            }
        }
    }
    if let Some(feature) = open {
        out.push(feature);
    }
    out
}

/// The declared feature names, `default` dropped.
pub fn feature_names(manifest: &str) -> Vec<String> {
    features(manifest)
        .into_iter()
        .map(|f| f.name)
        .filter(|n| n != "default")
        .collect()
}

/// `line` up to its first `#` that sits outside a string.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..i],
            _ => {}
        }
    }
    line
}

/// The double-quoted strings in `body`, quotes removed.
fn quoted(body: &str) -> Vec<String> {
    body.split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect()
}

/// The feature in `ours` (a crate depending on the core as `libviprs`) that
/// turns on the core's `feature`, skipping the bundle names in `bundles`.
///
/// Directly is `x = ["libviprs/feature"]`. A core feature that is a pure
/// alias of other core features (the core's deprecated
/// `s3 = ["object-store-sink"]`) also counts as turned on when everything it
/// aliases is, because enabling it changes nothing else: libviprs-tests'
/// `s3 = ["object-store-sink"]` reaches exactly the core's `s3`. A core
/// feature with any `dep:` or `crate/feature` entry is not an alias.
pub fn forwarder(
    ours: &[Feature],
    core: &[Feature],
    feature: &str,
    bundles: &[&str],
) -> Option<String> {
    let want = format!("libviprs/{feature}");
    if let Some(f) = ours.iter().find(|f| {
        f.name != "default" && !bundles.contains(&f.name.as_str()) && f.enables.contains(&want)
    }) {
        return Some(f.name.clone());
    }
    let def = core.iter().find(|f| f.name == feature)?;
    let pure_alias = !def.enables.is_empty()
        && def.enables.iter().all(|e| {
            e != feature
                && !e.contains('/')
                && !e.contains(':')
                && core.iter().any(|f| &f.name == e)
        });
    if !pure_alias {
        return None;
    }
    let mut first = None;
    for target in &def.enables {
        let name = forwarder(ours, core, target, bundles)?;
        first.get_or_insert(name);
    }
    first
}
