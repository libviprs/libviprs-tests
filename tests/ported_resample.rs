#![cfg(feature = "ported_tests")]

//! Phase 5 — Resampling tests ported from the test plan.
//!
//! Covers resize, affine transforms, and advanced resampling operations.

// ---------------------------------------------------------------------------
// 5.1 Resize (PARTIAL)
// ---------------------------------------------------------------------------
mod resize {
    #[test]
    fn test_resize_quarter() {
        let src = libviprs::source::generate_test_raster(256, 256).unwrap();
        let half = libviprs::resize::downscale_half(&src).unwrap();
        assert_eq!(half.width(), 128);
        assert_eq!(half.height(), 128);
        let quarter = libviprs::resize::downscale_half(&half).unwrap();
        assert_eq!(quarter.width(), 64);
        assert_eq!(quarter.height(), 64);
    }

    #[test]
    #[ignore]
    /// Verify downscale behaviour with odd-dimension inputs. Reference: vips_resize edge cases
    fn test_resize_rounding() {
        todo!("Not implemented: edge cases with odd dimensions need manual verification")
    }

    #[test]
    #[ignore]
    /// Shrink an image by an integer factor. Reference: vips_shrink
    fn test_shrink() {
        todo!("Not implemented: no explicit shrink API exposed in libviprs")
    }

    #[test]
    #[ignore]
    /// Shrink using box-average filter and compare quality. Reference: vips_shrink + averaging
    fn test_shrink_average() {
        todo!("Not implemented: shrink-average behaviour not verified in libviprs")
    }
}

// ---------------------------------------------------------------------------
// 5.2 Affine Transform (PARTIAL) — all manual / ignored
// ---------------------------------------------------------------------------
mod affine {
    #[test]
    #[ignore]
    /// Apply an affine rotation then its inverse; verify round-trip fidelity. Reference: vips_affine
    fn test_affine_rotation_roundtrip() {
        todo!("Not implemented: no affine transform API available in libviprs")
    }

    #[test]
    #[ignore]
    /// Rotate an image using the similarity operator. Reference: vips_similarity rotate
    fn test_similarity_rotate() {
        todo!("Not implemented: no similarity transform API available in libviprs")
    }

    #[test]
    #[ignore]
    /// Scale an image using the similarity operator. Reference: vips_similarity scale
    fn test_similarity_scale() {
        todo!("Not implemented: no similarity transform API available in libviprs")
    }

    #[test]
    #[ignore]
    /// Rotate an image by an arbitrary angle and check dimensions. Reference: vips_rotate
    fn test_rotate_arbitrary() {
        todo!("Not implemented: no arbitrary rotation API available in libviprs")
    }
}

// ---------------------------------------------------------------------------
// 5.3 Advanced Resampling (NOT IMPLEMENTED) — all manual / ignored
// ---------------------------------------------------------------------------
mod advanced_resampling {
    #[test]
    #[ignore]
    /// Compare output across different resampling kernels (bilinear, bicubic, lanczos). Reference: vips_reduce
    fn test_reduce_kernels() {
        todo!("Not implemented: no kernel-selectable reduce API available in libviprs")
    }

    #[test]
    #[ignore]
    /// Generate a thumbnail respecting aspect ratio. Reference: vips_thumbnail
    fn test_thumbnail() {
        todo!("Not implemented: no thumbnail API available in libviprs")
    }

    #[test]
    #[ignore]
    /// Generate a thumbnail that preserves the ICC profile. Reference: vips_thumbnail + ICC
    fn test_thumbnail_icc() {
        todo!("Not implemented: no thumbnail-with-ICC API available in libviprs")
    }

    #[test]
    #[ignore]
    /// Remap pixels via an index image. Reference: vips_mapim
    fn test_mapim() {
        todo!("Not implemented: no mapim API available in libviprs")
    }
}
