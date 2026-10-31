//! go-pmtiles reads the `vnd.pvec` key of the archive pvec-interchange exports for a raster of a
//! `.pvec` (libviprs-tests#305, libviprs/pvec#222). The oracle does not understand the key and has
//! to carry it through `show --metadata` untouched, which is what makes it a vendor key that
//! survives other people's tools. The GeoTransform key of an archive libviprs wrote is read the
//! same way in `pmtiles_interop.rs`. Runs in the `pmtiles-interop` job, where
//! `VIPRS_REQUIRE_GO_PMTILES=1` turns the skip into a failure. Needs `--features pvec-raster`.

#![cfg(feature = "pvec-raster")]

#[path = "common/pvec_raster.rs"]
mod fx;

use std::path::PathBuf;
use std::process::Command;

fn require_go_pmtiles() -> bool {
    std::env::var("VIPRS_REQUIRE_GO_PMTILES").is_ok_and(|v| v == "1")
}

/// The `pmtiles` binary, or None when it is not reachable and the caller did not demand it.
fn go_pmtiles_bin() -> Option<PathBuf> {
    let candidate = std::env::var_os("GO_PMTILES_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("pmtiles"));
    match Command::new(&candidate).arg("version").output() {
        Ok(out) if out.status.success() => Some(candidate),
        _ => {
            assert!(
                !require_go_pmtiles(),
                "VIPRS_REQUIRE_GO_PMTILES=1 but `{} version` did not run: this would skip the \
                 go-pmtiles comparison and report a false green",
                candidate.display()
            );
            None
        }
    }
}

#[test]
fn go_pmtiles_show_reads_the_vnd_pvec_key_of_the_export() {
    let Some(bin) = go_pmtiles_bin() else {
        eprintln!(
            "skipping: no go-pmtiles reachable (GO_PMTILES_BIN, or VIPRS_REQUIRE_GO_PMTILES=1)"
        );
        return;
    };
    let (_, pvec) = fx::interchange_pvec();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.pmtiles");
    std::fs::write(&path, fx::export_raster_0(&pvec)).unwrap();
    let out = Command::new(&bin)
        .args(["show", "--metadata"])
        .arg(&path)
        .output()
        .expect("run pmtiles show");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shown = String::from_utf8_lossy(&out.stdout).into_owned();
    let meta: serde_json::Value = serde_json::from_str(shown.trim())
        .unwrap_or_else(|e| panic!("`show --metadata` is not JSON ({e}): {shown}"));
    assert!(
        meta.get("vnd.pvec").is_some_and(|v| !v.is_null()),
        "no vnd.pvec in:\n{shown}"
    );
}
