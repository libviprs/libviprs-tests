//! Guards that CI runs `tests/codec_e2e.rs` with every codec feature on, and
//! cannot skip its way to a green (libviprs/libviprs-tests#230).
//!
//! `codec_e2e`'s JPEG XL, JPEG 2000, AVIF and SVG rows skip when the core was
//! built without the feature, and a skip reads to `cargo test` as a pass. The
//! default `cargo test` in the `test` job builds the core with none of the
//! four, so without a step of its own every one of those rows (the JPEG 2000
//! byte parity, the AVIF decode, the parked SVG limit cell) skips in CI and can
//! never go red. `CODEC_E2E_REQUIRE_FEATURES=all` turns the skip into a
//! failure, and this file stops that step, its features or its variable from
//! being deleted, moved or made optional without anything noticing.
//!
//! Like `tests/pmtiles_ci_wiring.rs`, this reads `ci.yml` only: no core codec,
//! no network, so it compiles against any counterpart.

mod common;
use common::workflows::{Job, Step, Workflow, read_workflow};

/// The job that carries the feature-on codec step.
const JOB: &str = "test";
/// The four non-default core features `codec_e2e` has rows for.
const CODEC_FEATURES: [&str; 4] = ["jxl", "jp2k", "avif", "svg"];

/// Whether this step runs `cargo test --test codec_e2e`, matched at a word
/// boundary so `--test codec_e2e_extra` does not count.
fn runs_codec_e2e(step: &Step) -> bool {
    let code = step.run_code();
    let needle = "--test codec_e2e";
    code.contains("cargo test")
        && code.match_indices(needle).any(|(i, _)| {
            code[i + needle.len()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        })
}

/// The features a `cargo test` line enables, from `--features "a b"`,
/// `--features=a,b` or `--features a`. `libviprs/x` counts as `x`: it turns the
/// same core feature on, and it is the spelling that works before this crate
/// forwards the feature under its own name.
fn features_of(step: &Step) -> Vec<String> {
    let code = step.run_code();
    let mut out = Vec::new();
    let mut rest = code.as_str();
    while let Some(at) = rest.find("--features") {
        rest = &rest[at + "--features".len()..];
        let list = rest.trim_start_matches(['=', ' ']);
        let list = if let Some(quoted) = list.strip_prefix('"') {
            quoted.split('"').next().unwrap_or("")
        } else {
            list.split_whitespace().next().unwrap_or("")
        };
        out.extend(
            list.split([' ', ','])
                .filter(|f| !f.is_empty())
                .map(|f| f.strip_prefix("libviprs/").unwrap_or(f).to_string()),
        );
    }
    out
}

fn codec_step(job: &Job) -> &Step {
    job.steps
        .iter()
        .find(|s| runs_codec_e2e(s))
        .unwrap_or_else(|| {
            panic!(
                "no step of `{JOB}` runs `cargo test --test codec_e2e`, so the \
                 JPEG XL, JPEG 2000, AVIF and SVG rows only ever run in the \
                 default build, where every one of them skips"
            )
        })
}

fn assert_a_step_runs_codec_e2e_with_every_codec_feature(ci: &Workflow) {
    let job = ci.job(JOB);
    let step = codec_step(job);
    let on = features_of(step);
    let missing: Vec<&str> = CODEC_FEATURES
        .iter()
        .copied()
        .filter(|f| !on.iter().any(|o| o == f))
        .collect();
    assert!(
        missing.is_empty(),
        "the codec_e2e step enables {on:?} and leaves out {missing:?}, so those \
         rows skip in CI. The step reads:\n{}",
        step.run()
    );
}

fn assert_the_step_cannot_skip_to_a_false_green(ci: &Workflow) {
    let job = ci.job(JOB);
    let step = codec_step(job);
    let set = job.env_for(step, "CODEC_E2E_REQUIRE_FEATURES");
    assert_eq!(
        set,
        Some("all"),
        "the codec_e2e step sees CODEC_E2E_REQUIRE_FEATURES as {set:?}. It has \
         to be `all`, on that step or its job, so a build that lost a feature \
         fails instead of skipping the rows and reporting a pass"
    );
}

fn assert_the_step_is_not_optional(ci: &Workflow) {
    let job = ci.job(JOB);
    for (what, key) in [("job", job.key("if")), ("step", codec_step(job).key("if"))] {
        assert!(
            key.is_none(),
            "the codec_e2e {what} carries `if: {}`, so whether it runs depends \
             on a condition, and a step that does not run reports no failures",
            key.unwrap_or_default()
        );
    }
    for (what, tolerated) in [
        ("job", job.key("continue-on-error")),
        ("step", codec_step(job).key("continue-on-error")),
    ] {
        let tolerated = tolerated.unwrap_or("false");
        assert_eq!(
            tolerated, "false",
            "the codec_e2e {what} sets continue-on-error: {tolerated}, which is \
             the same green as never running it"
        );
    }
}

type Guard = (&'static str, fn(&Workflow));

const GUARDS: &[Guard] = &[
    (
        "a step runs codec_e2e with every codec feature",
        assert_a_step_runs_codec_e2e_with_every_codec_feature,
    ),
    (
        "the step cannot skip to a false green",
        assert_the_step_cannot_skip_to_a_false_green,
    ),
    ("the step is not optional", assert_the_step_is_not_optional),
];

fn all_guards_pass(text: &str) -> Result<(), String> {
    let ci = Workflow::parse(text);
    for (name, guard) in GUARDS {
        let ci = ci.clone();
        if std::panic::catch_unwind(move || guard(&ci)).is_err() {
            return Err((*name).to_string());
        }
    }
    Ok(())
}

#[test]
fn ci_runs_codec_e2e_with_every_codec_feature_and_no_skips() {
    let ci = Workflow::parse(&read_workflow("ci.yml"));
    for (name, guard) in GUARDS {
        eprintln!("checking: {name}");
        guard(&ci);
    }
}

/// Each edit below is one GitHub would honour, and one of the guards has to
/// refuse it. Applied to the real `ci.yml`, with every anchor required to
/// match exactly once so an edit cannot land somewhere else and blame the
/// guards for a mutation they never saw.
#[test]
fn the_guards_red_on_the_edits_they_exist_to_catch() {
    let ci = read_workflow("ci.yml");
    assert_eq!(
        all_guards_pass(&ci),
        Ok(()),
        "the guards must pass on the real file before the mutations mean anything"
    );
    let step = "--test codec_e2e";
    let env = "CODEC_E2E_REQUIRE_FEATURES: all";
    let mutations: [(&str, &str, &str); 6] = [
        ("step deleted", step, "--test codec_e2e_renamed"),
        ("env weakened", env, "CODEC_E2E_REQUIRE_FEATURES: jxl"),
        ("env deleted", env, "CODEC_E2E_NOT_REQUIRED: all"),
        ("avif dropped", "\"jxl avif svg jp2k\"", "\"jxl svg jp2k\""),
        ("svg dropped", "\"jxl avif svg jp2k\"", "\"jxl avif jp2k\""),
        (
            "step made tolerant",
            "run: cargo test --features \"jxl avif",
            "continue-on-error: true\n        run: cargo test --features \"jxl avif",
        ),
    ];
    for (name, anchor, replacement) in mutations {
        assert_eq!(
            ci.matches(anchor).count(),
            1,
            "mutation `{name}`: anchor {anchor:?} must match ci.yml exactly once"
        );
        let mutated = ci.replacen(anchor, replacement, 1);
        assert!(
            all_guards_pass(&mutated).is_err(),
            "mutation `{name}` left every guard green"
        );
    }
}
