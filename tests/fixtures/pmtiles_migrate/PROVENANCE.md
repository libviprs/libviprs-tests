# `blueprint-portrait-google-centre.pmtiles` (libviprs-tests#262)

This archive did not come from go-pmtiles, so it is not an oracle answer, and it lives here rather
than in `tests/fixtures/pmtiles/` because `pmtiles_ci_wiring.rs` requires everything in that
directory to be described by the go-pmtiles vectors. It is the stored output of the migration iasbuilt/server runs: page 1 of
`blueprint-portrait.pdf` (3300x5024) extracted with `extract_page_image`, planned as
`Layout::Google` with `.with_centre(true)` at tile size 256, written to a loose-file tree with
`FsSink` (PNG), then moved into this archive with `migrate_directory_to_pmtiles`. 4170264 bytes,
sha256 `a21cc6ea0edfcd6ca8e59e7ee41f11b26d47b486b0b0751816e595670b909fd7`.

It pins what the migration produces so a change shows up as a diff against a file. It cannot say the
mapping is right, because the library wrote it. That is the job of
`a_migrated_google_tree_puts_every_tile_at_zxy_with_its_own_bytes`, which compares the archive with
the `{z}/{row}/{col}.png` files directly.

To regenerate, which needs this file and `GOLDEN_SHA256` in `tests/pmtiles_migrate_google.rs`
updated together:

```
cargo test --test pmtiles_migrate_google -- --ignored write_the_migrate_golden --nocapture
```

