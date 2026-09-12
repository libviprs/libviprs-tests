# go-pmtiles fixtures

Every byte under this directory was written by the real `protomaps/go-pmtiles`. Nothing here came
out of libviprs, and nothing here was derived by reading the PMTiles v3 spec and doing the
arithmetic by hand. That is the whole point of the directory: `tests/pmtiles_interop.rs` pins our
reader against an implementation that has never seen our code, and a fixture we generated ourselves
would prove only that we agree with ourselves.

## The pin

| | |
|---|---|
| Repository | `protomaps/go-pmtiles` |
| Release tag | `v1.31.2` |
| Source commit | `a3e4951ea6a0477b784c27c1dcbfd9c130878c5a` |
| `pmtiles version` says | `pmtiles 1.31.2, commit a3e4951ea6a0477b784c27c1dcbfd9c130878c5a, built at 2026-07-22T18:59:03Z` |

The release carries no `checksums.txt`, so the tarball digests below are measurements, each
cross-checked against the size the GitHub API reports for that asset.

| Asset | Bytes | sha256 |
|---|---|---|
| `go-pmtiles_1.31.2_Linux_x86_64.tar.gz` | 17444324 | `3ed7dbf4ec2e6dfe5e25b6f70d1ffc932729f93c86db353bf514dd71010a312f` |
| `go-pmtiles_1.31.2_Linux_arm64.tar.gz` | 15779781 | `f8bd47e7ea866863489cad588fbaf2f31f42e5821f7a03f009b3769f05801cb1` |

| Inner `pmtiles` binary | sha256 |
|---|---|
| linux/amd64 | `a7e9ae10184d109c83f456ccdf6df4f3e2a64ba6cf69d9ed0f9f1840305055c1` |
| linux/arm64 | `8cd0affde1ba5380b7cea6de0f94c674f88e4f586c77ae5820ea9652862691f4` |

`.github/workflows/ci.yml` downloads the x86_64 tarball and verifies it with `sha256sum -c`, because
`ubuntu-latest` runners are x86_64. `tests/pmtiles_interop.rs` asserts the release tag CI pins is the
same one the vector files below say produced them, so the two cannot drift apart.

## The archives

| Archive | Bytes | sha256 |
|---|---|---|
| `raster-z0z2.pmtiles` | 1878 | `e2ed5e64f3c29efa3ec3b679ec5f1b06569c1b234c6eea762fb9f02fc23e9c12` |
| `dupes-z0z3.pmtiles` | 5007 | `bfc9db4c6ce6a04194e02b3d4815814adb05209f1aaba8591e4e1332f6e56a27` |
| `leaves-z0z7.pmtiles` | 869 | `fe5c9636be61abc60046d7f13837f8a3efb20ce3c38303644dac0cbec8248b8d` |

All three pass `pmtiles verify` with exit code 0.

`go-pmtiles` cannot synthesise an archive from nothing: `convert` is its only subcommand that builds
one from raw tiles and its only input is an MBTiles database. So the tile *payloads* were written by
a small Python PNG encoder and the *archive layout* is entirely go-pmtiles'. That split matters when
you read a failure: a disagreement about bytes is about the payload, a disagreement about where the
bytes live is about the format.

**`raster-z0z2.pmtiles`** is the plain one. Zoom 0 to 2, 21 tiles, every tile a distinct 8x8 solid
PNG so nothing can deduplicate. One root directory, no leaves.

**`dupes-z0z3.pmtiles`** carries both duplicate shapes, which is the point of it. Runs, where the
four z1 tiles and the sixteen z2 tiles each collapse into one entry with `run_length` 4 and 16; and
repeated offsets that are *not* adjacent, where entries at tile ids 21, 49, 63 and 76 all point at
offset 148 with `run_length` 1 each. A reader that handles only the first shape returns wrong bytes
for the second and still looks fine.

**`leaves-z0z7.pmtiles`** is the one that earns its place. It has 6 real leaf directories over 21844
entries and only 2 tile payloads, and as far as anyone on this epic has found it is the only fixture
anywhere with leaves at all: none of the three fixtures in the upstream spec repository has any. So
nothing upstream exercises the leaf entry offset base, and a writer and a reader that make the same
wrong choice about it round-trip perfectly.

The base is the thing to get right. A tile entry found inside a leaf is still relative to
`tile_data_offset`, not to the leaf's own start and not to `leaf_directory_offset`. Here
`leaf_directory_offset` is 334, `tile_data_offset` is 725 and `tile_data_length` is 144, and every
tile entry in every leaf carries offset 0 or 72, landing at absolute 725 or 797. Under the leaf's own
start the same offsets land at 405 and 477, inside the directory region, so a reader with the wrong
base hands back directory bytes dressed as a tile. A test that only asserts `get_tile` returned
`Some` passes either way, which is why `leaf_entries_resolve_against_tile_data_not_the_leaf_start`
compares the payload and shows what the wrong base would have returned.

## The vectors

`vectors/tiles.json` and `vectors/header.json` are dumps from the same pinned binary, copied
unchanged from `.epicF/oracle/vectors/`. `tiles.json` records, for a handful of `(z, x, y)` per
archive, the payload length and sha256 `pmtiles tile` wrote, plus rows for tiles the archive does not
hold and rows the reference answers *wrongly*. `header.json` records each archive's raw 127 header
bytes and the field-by-field decoding `pmtiles.DeserializeHeader` gives for them.

The `out_of_range` rows are evidence about the reference, not a target. `ZxyToID` masks an x or y
past `2**z - 1` into a different, valid tile and serves it with exit 0, so go-pmtiles answers
`(z=2, x=4, y=0)` with the payload of a real tile. libviprs refuses those coordinates, and
`tests/pmtiles_interop.rs` asserts the refusal. Matching the reference there would be matching a bug.

## Regenerating

`.epicF/oracle/` holds `Dockerfile.oracle`, which downloads the release and verifies it with
`sha256sum -c` at build time, plus the generator scripts and the full write-up in its own
`PROVENANCE.md` and `goldens/PROVENANCE.md`. Start there rather than from this file.
