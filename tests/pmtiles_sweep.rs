//! Every cell of every committed golden, against what the reference answered
//! for it (libviprs-tests#202, EPIC F libviprs/libviprs#986).
//!
//! # Why a sweep and not a probe set
//!
//! `vectors/tiles.json` carries a handful of hand-picked `(z, x, y)` rows per
//! archive, and a review that mutated the reader measured what those rows can
//! actually see: the whole suite's defence against a dropped Hilbert rotation
//! was **one** row, `dupes (3, 3, 5)`. Reorder `tiles.json` and the repository
//! stops being able to see that mistake at all while every test still runs and
//! still passes.
//!
//! A hand-picked probe set has a second failure mode that is worse, because it
//! looks like diligence. Four probes in the external interop cell were
//! `(0,0)`, `(1,0)`, `(0,1)` and the far corner of a 4x4 level, and all four
//! are fixed points of every plausible tile-id mistake, so the number of probes
//! was not the problem and adding more of the same would not have helped. The
//! same shape shows up across the field: `pmtiles-rs` makes the best
//! cross-implementation assertion published anywhere and picks `(0,0,0)`,
//! `(2,2,2)` and `(3,4,5)`, which are all fixed points, while 32 of that
//! archive's 85 coordinates would have caught it.
//!
//! So this walks every cell of every zoom each archive declares, in one pass,
//! and compares a digest per zoom against one the reference produced. What it
//! costs is a 15 KB vector file; what it buys is that no future edit can
//! quietly narrow the coverage, because the coverage is "all of it".
//!
//! # What the digests are
//!
//! For each zoom, every cell in raster order contributes one line: `z/x/y` then
//! either `absent` or the sha256 of the payload. The digest is the sha256 of
//! those lines. `vectors/sweep.json` holds one per zoom, produced by running
//! `pmtiles tile` against the committed archive for all 44174 cells, so a
//! disagreement is a disagreement with the reference and not with a number this
//! repository made up.
//!
//! A per-zoom digest localises a failure to a zoom and no further, which is the
//! trade for the file staying small. `discriminating` in the same file carries
//! a few dozen full rows per archive for the cells most mistakes move, so the
//! common failures still name a coordinate.

use std::collections::BTreeMap;

mod common;

#[path = "common/pmtiles.rs"]
mod pmtiles_support;
use pmtiles_support::{
    GOLDENS, WrongConvention, golden_path, sha256_hex, sweep_vectors, what_a_correct_reader_finds,
};

use libviprs::pmtiles::{FileRangeReader, Reader};

/// Every cell of one zoom, as `(x, y) -> digest of the payload`, with absent
/// cells left out rather than recorded as empty.
///
/// Left out rather than empty because an empty vector and a missing tile are
/// different answers and a map that conflates them cannot tell a reader that
/// returns `Some(vec![])` everywhere from a correct one.
type Level = BTreeMap<(u32, u32), String>;

fn walk(reader: &Reader<FileRangeReader>, z: u8) -> (Level, String) {
    let side: u32 = 1 << z;
    let mut level = Level::new();
    let mut lines = String::new();
    for y in 0..side {
        for x in 0..side {
            let answer = reader
                .get_tile(z, x, y)
                .unwrap_or_else(|e| panic!("({z}, {x}, {y}): {e}"));
            match answer {
                Some(bytes) => {
                    let digest = sha256_hex(&bytes);
                    lines.push_str(&format!("{z}/{x}/{y} {digest}\n"));
                    level.insert((x, y), digest);
                }
                None => lines.push_str(&format!("{z}/{x}/{y} absent\n")),
            }
        }
    }
    (level, sha256_hex(lines.as_bytes()))
}

/// Every cell of every zoom has to answer what `pmtiles tile` answered.
#[test]
fn every_cell_of_every_golden_answers_what_the_reference_answered() {
    let sweep = sweep_vectors();
    let mut cells = 0u64;
    let mut zooms = 0usize;

    for (name, digest) in GOLDENS {
        let reader = Reader::try_open(golden_path(name, digest))
            .unwrap_or_else(|e| panic!("open {name}: {e}"));
        let header = reader.header();
        let recorded = sweep_zooms(&sweep, name);
        assert_eq!(
            recorded.keys().copied().collect::<Vec<u8>>(),
            (header.min_zoom..=header.max_zoom).collect::<Vec<u8>>(),
            "{name}: the vector file records zooms {:?} and the header declares \
             {}..={}",
            recorded.keys().collect::<Vec<&u8>>(),
            header.min_zoom,
            header.max_zoom
        );

        for (z, want) in &recorded {
            let (level, got) = walk(&reader, *z);
            assert_eq!(
                &got,
                &want.digest,
                "{name} zoom {z}: the {} cells of this level do not answer what \
                 the reference answered. {} of them hold a tile here and the \
                 reference recorded {}.",
                1u64 << (2 * u32::from(*z)),
                level.len(),
                want.present
            );
            assert_eq!(
                level.len() as u64,
                want.present,
                "{name} zoom {z}: we hold {} tiles and the reference found {}",
                level.len(),
                want.present
            );
            cells += 1u64 << (2 * u32::from(*z));
            zooms += 1;
        }
    }

    assert!(
        zooms >= 24 && cells >= 40_000,
        "the sweep covered {cells} cells over {zooms} zooms, which is less than \
         the committed goldens declare"
    );
}

/// Some committed golden has to be able to see each wrong tile-id convention,
/// in each structural shape the archives come in.
///
/// # The measurement this exists because of
///
/// `leaves-z0z7` was the only golden with leaf directories, and it cannot see
/// any tile-id mistake at all. Its two payloads alternate by `(x + y) % 2`, and
/// every symmetric convention error preserves that parity at every zoom, so all
/// 21845 cells come back identical under five wrong conventions. A fixture with
/// 21845 tiles that proves nothing about where any of them is reads as thorough
/// coverage right up until somebody checks.
///
/// The reflection error being parity-preserving is not bad luck specific to
/// that fixture: it flips the parity of 0 cells out of 65536 at zoom 8. Any
/// two-payload checkerboard would have been just as blind.
///
/// So this measures, rather than assuming, and it insists the coverage exists
/// separately for archives with leaves and archives without, because the leaf
/// lookup path is the one where a wrong convention and a wrong leaf base are
/// hard to tell apart.
#[test]
fn a_golden_of_each_shape_can_see_every_wrong_tile_id_convention() {
    let mut best: BTreeMap<(bool, &str), (usize, &str)> = BTreeMap::new();

    for (name, digest) in GOLDENS {
        let reader = Reader::try_open(golden_path(name, digest))
            .unwrap_or_else(|e| panic!("open {name}: {e}"));
        let header = reader.header();
        let has_leaves = header.has_leaves();
        // The deepest zoom is where a permutation moves the most, and walking
        // one level is enough to establish whether the fixture can see it.
        let z = header.max_zoom;
        let (level, _) = walk(&reader, z);

        for wrong in WrongConvention::ALL {
            let finds = what_a_correct_reader_finds(z, 1u32 << z, *wrong);
            // The fixture sees the mistake at a cell when the payload a correct
            // reader would find there is not the payload that belongs there.
            let caught = finds
                .iter()
                .filter(|(asked, found)| asked != found && level.get(*asked) != level.get(*found))
                .count();
            let slot = best.entry((has_leaves, wrong.name())).or_insert((0, name));
            if caught > slot.0 {
                *slot = (caught, name);
            }
        }
    }

    let mut blind = Vec::new();
    for ((has_leaves, wrong), (caught, name)) in &best {
        let shape = if *has_leaves {
            "with leaf directories"
        } else {
            "root-only"
        };
        eprintln!("{shape}, {wrong}: best is {name} at {caught} cells");
        if *caught == 0 {
            blind.push(format!("{shape}: nothing here can see `{wrong}`"));
        }
    }
    assert!(
        blind.is_empty(),
        "{blind:#?}\nA committed archive that cannot tell a wrong tile-id \
         convention from the right one is not evidence about the convention, \
         however many tiles it holds."
    );
}

/// One zoom's row out of `vectors/sweep.json`.
struct Zoom {
    digest: String,
    present: u64,
}

fn sweep_zooms(vectors: &serde_json::Value, archive: &str) -> BTreeMap<u8, Zoom> {
    let block = vectors
        .get("archives")
        .and_then(|a| a.get(archive))
        .unwrap_or_else(|| panic!("sweep.json has no block for {archive}"));
    let zooms = block
        .get("zooms")
        .and_then(|z| z.as_array())
        .unwrap_or_else(|| panic!("{archive} has no zooms array in sweep.json"));
    zooms
        .iter()
        .map(|row| {
            let z = row.get("z").and_then(serde_json::Value::as_u64).expect("z") as u8;
            (
                z,
                Zoom {
                    digest: row
                        .get("digest")
                        .and_then(serde_json::Value::as_str)
                        .expect("digest")
                        .to_string(),
                    present: row
                        .get("present")
                        .and_then(serde_json::Value::as_u64)
                        .expect("present"),
                },
            )
        })
        .collect()
}
