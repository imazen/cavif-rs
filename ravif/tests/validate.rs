//! Per-variant + happy-path coverage for [`zenravif::Encoder::validate`].
//!
//! See `src/validate.rs` for source citations of each accepted range.

use zenravif::{ChromaSubsampling, ColorModel, Encoder, ValidationError};

#[test]
fn happy_path_default_validates_ok() {
    let enc = Encoder::new();
    assert_eq!(enc.validate(), Ok(()));
}

#[test]
fn happy_path_typical_config_validates_ok() {
    let enc = Encoder::new()
        .with_quality(80.0)
        .with_alpha_quality(90.0)
        .with_speed(5)
        .with_num_threads(Some(4));
    assert_eq!(enc.validate(), Ok(()));
}

#[test]
fn happy_path_libavif_quality_validates_ok() {
    let enc = Encoder::new().with_libavif_quality(50.0);
    assert_eq!(enc.validate(), Ok(()));
}

#[test]
fn quality_above_range_rejected() {
    let enc = Encoder::new().with_quality(150.0);
    let err = enc.validate().unwrap_err();
    assert!(matches!(err, ValidationError::QualityOutOfRange { value, .. } if value == 150.0));
}

#[test]
fn quality_below_range_rejected() {
    let enc = Encoder::new().with_quality(0.5);
    let err = enc.validate().unwrap_err();
    assert!(matches!(err, ValidationError::QualityOutOfRange { value, .. } if value == 0.5));
}

#[test]
fn alpha_quality_out_of_range_rejected() {
    let enc = Encoder::new().with_alpha_quality(150.0);
    assert!(matches!(
        enc.validate(),
        Err(ValidationError::AlphaQualityOutOfRange { .. })
    ));
}

#[test]
fn libavif_quality_out_of_range_rejected() {
    let enc = Encoder::new().with_libavif_quality(150.0);
    assert!(matches!(
        enc.validate(),
        Err(ValidationError::LibavifQualityOutOfRange { .. })
    ));
}

#[test]
fn speed_above_range_rejected() {
    let enc = Encoder::new().with_speed(20);
    assert!(matches!(
        enc.validate(),
        Err(ValidationError::SpeedOutOfRange { value: 20, .. })
    ));
}

#[test]
fn speed_zero_rejected() {
    let enc = Encoder::new().with_speed(0);
    assert!(matches!(
        enc.validate(),
        Err(ValidationError::SpeedOutOfRange { value: 0, .. })
    ));
}

#[test]
fn num_threads_zero_rejected() {
    let enc = Encoder::new().with_num_threads(Some(0));
    assert_eq!(enc.validate(), Err(ValidationError::NumThreadsZero));
}

#[test]
fn num_threads_none_validates_ok() {
    let enc = Encoder::new().with_num_threads(None);
    assert_eq!(enc.validate(), Ok(()));
}

#[test]
fn rotation_above_range_rejected() {
    let enc = Encoder::new().with_rotation(4);
    assert!(matches!(
        enc.validate(),
        Err(ValidationError::RotationOutOfRange { value: 4, .. })
    ));
}

#[test]
fn rotation_in_range_validates_ok() {
    for r in 0..=3 {
        let enc = Encoder::new().with_rotation(r);
        assert_eq!(enc.validate(), Ok(()), "rotation={r} should be valid");
    }
}

#[test]
fn mirror_above_range_rejected() {
    let enc = Encoder::new().with_mirror(2);
    assert!(matches!(
        enc.validate(),
        Err(ValidationError::MirrorOutOfRange { value: 2, .. })
    ));
}

#[test]
fn mirror_in_range_validates_ok() {
    for m in 0..=1 {
        let enc = Encoder::new().with_mirror(m);
        assert_eq!(enc.validate(), Ok(()), "mirror={m} should be valid");
    }
}

#[test]
fn yuv420_plus_rgb_rejected() {
    let enc = Encoder::new()
        .with_chroma_subsampling(ChromaSubsampling::Yuv420)
        .with_internal_color_model(ColorModel::RGB);
    assert!(matches!(
        enc.validate(),
        Err(ValidationError::MutuallyExclusive { .. })
    ));
}

#[test]
fn yuv420_plus_ycbcr_validates_ok() {
    let enc = Encoder::new()
        .with_chroma_subsampling(ChromaSubsampling::Yuv420)
        .with_internal_color_model(ColorModel::YCbCr);
    assert_eq!(enc.validate(), Ok(()));
}

#[test]
fn yuv444_plus_rgb_validates_ok() {
    let enc = Encoder::new()
        .with_chroma_subsampling(ChromaSubsampling::Yuv444)
        .with_internal_color_model(ColorModel::RGB);
    assert_eq!(enc.validate(), Ok(()));
}

#[cfg(feature = "imazen")]
mod imazen_only {
    use super::*;

    #[test]
    fn vaq_strength_above_range_rejected() {
        let enc = Encoder::new().with_vaq(true, 5.0);
        assert!(matches!(
            enc.validate(),
            Err(ValidationError::VaqStrengthOutOfRange { .. })
        ));
    }

    #[test]
    fn vaq_strength_below_range_rejected() {
        let enc = Encoder::new().with_vaq(true, -0.1);
        assert!(matches!(
            enc.validate(),
            Err(ValidationError::VaqStrengthOutOfRange { .. })
        ));
    }

    #[test]
    fn vaq_strength_in_range_validates_ok() {
        let enc = Encoder::new().with_vaq(true, 2.0);
        assert_eq!(enc.validate(), Ok(()));
    }

    #[test]
    fn seg_boost_above_range_rejected() {
        let enc = Encoder::new().with_seg_boost(5.0);
        assert!(matches!(
            enc.validate(),
            Err(ValidationError::SegBoostOutOfRange { .. })
        ));
    }

    #[test]
    fn seg_boost_below_range_rejected() {
        let enc = Encoder::new().with_seg_boost(0.25);
        assert!(matches!(
            enc.validate(),
            Err(ValidationError::SegBoostOutOfRange { .. })
        ));
    }

    #[test]
    fn seg_boost_default_one_validates_ok() {
        // 1.0 is the no-op default and is always accepted.
        let enc = Encoder::new().with_seg_boost(1.0);
        assert_eq!(enc.validate(), Ok(()));
    }

    #[test]
    fn seg_boost_in_range_validates_ok() {
        let enc = Encoder::new().with_seg_boost(2.0);
        assert_eq!(enc.validate(), Ok(()));
    }
}

#[cfg(feature = "__expert")]
mod expert_only {
    use super::*;
    use zenravif::expert::InternalParams;

    fn with_partition(min: u8, max: u8) -> Encoder<'static> {
        let mut p = InternalParams::default();
        p.partition_range = Some((min, max));
        Encoder::new().with_internal_params(p)
    }

    #[test]
    fn partition_min_invalid_size_rejected() {
        let enc = with_partition(3, 16);
        assert!(matches!(
            enc.validate(),
            Err(ValidationError::PartitionRangeInvalid { min: 3, max: 16 })
        ));
    }

    #[test]
    fn partition_max_invalid_size_rejected() {
        let enc = with_partition(8, 128);
        assert!(matches!(
            enc.validate(),
            Err(ValidationError::PartitionRangeInvalid { min: 8, max: 128 })
        ));
    }

    #[test]
    fn partition_min_greater_than_max_rejected() {
        let enc = with_partition(32, 8);
        assert!(matches!(
            enc.validate(),
            Err(ValidationError::PartitionRangeInvalid { min: 32, max: 8 })
        ));
    }

    #[test]
    fn partition_valid_range_validates_ok() {
        for &min in &[4u8, 8, 16, 32, 64] {
            for &max in &[4u8, 8, 16, 32, 64] {
                if min > max {
                    continue;
                }
                let enc = with_partition(min, max);
                assert_eq!(
                    enc.validate(),
                    Ok(()),
                    "partition ({min}, {max}) should validate"
                );
            }
        }
    }

    #[test]
    fn partition_none_validates_ok() {
        let enc = Encoder::new().with_internal_params(InternalParams::default());
        assert_eq!(enc.validate(), Ok(()));
    }
}
