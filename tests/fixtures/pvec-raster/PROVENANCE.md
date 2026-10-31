# PVec raster fixtures

These are the inputs of the raster-into-PVec suites (libviprs-tests#305, EPIC R-L, libviprs/libviprs#1329).
I made every file here with a function in `tests/common/pvec_raster.rs`, called by `tests/gen_source_rasters.rs`.
Nothing is downloaded and nothing is photographed or scanned, so there is no third party licence to carry:
the pixels come from arithmetic, a fixed LCG seed and nothing else, and the files are mine to give away under the
repo's licence.

`SHA256SUMS` lists the digest of each file. Two tests keep it honest. `pvec_raster_fixtures_regenerate_to_their_digests`
regenerates every file in memory and compares it with the committed bytes and with `SHA256SUMS`, and
`a_fixture_with_one_flipped_byte_fails_its_sha256sums_check` flips one byte of each and checks the digest notices.
To regenerate after a deliberate change (a bump of `image`, `lopdf` or the PVec pin changes the bytes):

    cargo test --test gen_source_rasters -- --ignored write_pvec_raster_fixtures

| file | what it is | why it is shaped that way |
|---|---|---|
| `scan_gray_1537x1031.png` | 8-bit gray, 1,537 by 1,031: paper tone, a ruled grid, a diagonal, scattered dots, and a blank (all 255) quarter from (768, 512) | odd sizes, so the last column and row of 256 px tiles pad (a 7 by 5 grid, 255 and 249 pixels of margin); the blank quarter holds whole blank tiles (columns 3 to 5, rows 2 and 3) for `skip_blanks` |
| `scan_1bit_1537x1031.png` | the same scan thresholded at 128, stored as a real 1-bit PNG | the bilevel case. It decodes to 8-bit gray with only 0 and 255, which is what the producer cuts |
| `photo_rgb_700x500.png` | 8-bit RGB gradient with concentric rings | JPEG tiles (bytes are compared, pixels are not) |
| `sheet_rgba_600x400.png` | 8-bit RGBA, opaque inside x 37..563, y 11..380 (one corner chamfered), transparent black outside | a clip baked into alpha. The box asymmetric (37 off the left and right, 11 off the top, 20 off the bottom), and it is the `visible_box` `[37, 11, 563, 380]` the clipped fixture of libviprs/maplibre-pvec#102 draws |
| `vector_gray_1024x512.png` | 8-bit gray ramp, a black 40 px marker in the top-left corner | the test vector of `docs/RASTER.md` section 5 in libviprs/pvec: placement `[0.5 0.25 0.125 -0.5 100 50]`, every number exact in binary |
| `point_gray_300x200.png` and `.pgw` | a small gray XOR pattern and its world file `0.5 0 0 -0.5 100.25 199.75` | the half-pixel shift. A world file names the centre of the upper-left pixel (`PixelIsPoint`), so the corner placement is `[0.5 0 0 -0.5 100 200]`; read as a corner (`PixelIsArea`) it is `[0.5 0 0 -0.5 100.25 199.75]` |
| `three_images.pdf` | one page, 612 by 792, with two image XObjects (4 by 4 gray, 3 by 2 RGB) and one inline image (2 by 2 gray) | the count of image constructs in libviprs/libviprs#1328 and #1332 is three, written plus refused |
| `drawing.pvec` | a PVec artifact holding one closed outline and no raster | the vector drawing a raster rides (WRT-01 refuses a file with no entity), and the raster-free twin that the degraded-open test compares geometry with |

A GeoTIFF with `PixelIsPoint` is not committed: nothing in this tree writes one, and the half-pixel case is the same
arithmetic as the world file's, which libviprs takes as `PixelOrigin::Centre`.

`drawing.pvec` is written by the PVec facade the core pins (`libviprs/pvec` at b9f4416), so its bytes move with that pin.
