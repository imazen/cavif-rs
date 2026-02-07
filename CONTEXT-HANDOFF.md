# Investigation Handoff: ravif vs avifenc Size/Quality Gap

**Date:** 2026-02-07
**Session:** Quality curve validation and benchmarking
**Status:** Ready for deeper investigation

---

## Problem Statement

Comprehensive testing on CLIC2025 corpus (5 real photographic images, Q5-Q95) revealed:

1. **30-40% quality gap at same Q number** (avifenc consistently higher quality)
2. **Ravif is LESS efficient at matched quality** (needs larger files to match avifenc's quality)
3. **Default ravif curve creates 20-point Q equivalence gap** (Q70 default ≈ Q50 avifenc)

This contradicts prior findings suggesting rav1e is "5-15% more efficient" and needs investigation.

---

## Data Summary

### Quality Gap by Range (Butteraugli Error)

| Q Range | Avg BA Ratio | Quality Difference |
|---------|---|---|
| Q5-Q20 | 0.89 | avifenc 11% better |
| Q25-Q45 | 0.66 | avifenc 34% better |
| Q50-Q75 | 0.69 | avifenc 31% better |
| Q80-Q95 | 0.82 | avifenc 18% better |

**Worst case:** Q30-Q75 range shows 34% quality advantage to avifenc.

### File Size Comparison at Matched Quality

Example (CLIC image 02809272b4ca9b08):

| Quality Level | Ravif File | Avifenc File | Ravif Overhead |
|---|---|---|---|
| BA ~3.8 | Q70 (153 KB) | Q50 (117 KB) | +31% |
| BA ~2.0 | Q90 (390 KB) | Q70 (211 KB) | +85% |

**Paradox:** Ravif is LESS efficient at matched quality, contradicting "rav1e superiority" hypothesis.

---

## Tested Scenarios

### ✅ Completed

1. **Synthetic image testing** (256x256 plasma)
   - Result: Butteraugli metrics unreliable on synthetic content
   - Decision: Dismissed as "worthless"

2. **Real CLIC2025 testing** (1024x1024 photographic)
   - 5 images × 19 quality levels = 95 encode/decode/measure cycles
   - Butteraugli metric used (lower error = higher quality)
   - Full dataset: `/mnt/v/output/ravif/full_quality_sweep.csv`

3. **Quality curve validation**
   - Confirmed default ravif uses 3-segment curve (aggressive)
   - Confirmed libavif uses linear curve (standard AVIF)
   - Curves are parallel offsets, not shape differences

### ❓ Unexplored

1. **Encoder configuration differences**
   - What speed settings were used? (both speed=6)
   - What encoder presets? (default for both)
   - What color space? (YCbCr default for both)
   - What threading/parallelism?

2. **Quantizer-to-quality mapping investigation**
   - How does rav1e respond to qindex 76 vs 107?
   - Is quality degradation linear across quantizer range?
   - Do AV1 transform/partition strategies differ at same qindex?

3. **Entropy/RDO differences**
   - Are libavif and ravif using different rate-distortion optimization?
   - Different entropy coding thresholds?
   - Different CRF/quality tuning curves internally?

4. **Frame-level analysis**
   - What's the actual frame size at each quantizer?
   - Tile overhead differences?
   - Metadata overhead?

5. **rav1e efficiency claim verification**
   - Original claim: "5-15% more efficient at matched quality"
   - Source: Earlier cid22 corpus investigation (not in current session)
   - Does it hold for CLIC2025? (appears NOT to)
   - Were those using default or libavif-scale? (unclear)

---

## Hypotheses to Test

### H1: Encoder Speed Settings
**Theory:** Speed 6 may not be optimal for quality comparison
- rav1e speed 6 ≈ "fast but quality-focused"
- avifenc default ≈ possibly different speed preset
- **Test:** Encode same images at speed 1, 4, 6, 8 and compare quality curves

### H2: Quantizer Sensitivity Difference
**Theory:** rav1e and libaom have different quality response at same quantizer
- rav1e qindex 76 → higher quality than libaom qindex 76?
- Or lower quality? (current data suggests lower)
- **Test:** Encode at qindex 50, 76, 100, 150 (varying quantizers, not Q mapping)

### H3: Rate-Distortion Tuning
**Theory:** avifenc uses different internal quality tuning
- libavif may have quality bias toward higher quality
- ravif may optimize for efficiency over perceived quality
- **Test:** Check rav1e/aom source for quality/efficiency tuning parameters

### H4: Default Curve Intentional Design
**Theory:** Ravif's aggressive curve is deliberate optimization choice
- Maybe default curve balances file size vs quality for "typical" users
- Libavif-scale matches spec, but isn't optimal for many use cases
- **Test:** Compare with other codec curves (libjxl, JPEG XL, etc.)

### H5: Test Methodology Artifact
**Theory:** Butteraugli may not capture quality differences accurately
- Maybe SSIMULACRA2 shows different results?
- Maybe VMAF/other metrics tell different story?
- **Test:** Re-run same encodings, measure with SSIMULACRA2 and VMAF

---

## Files to Investigate

### In Repository

- **`ravif/src/av1encoder.rs`**
  - Line 677: `quality_to_quantizer()` - default 3-segment curve definition
  - Line 203-218: `with_libavif_quality()` - libavif-scale implementation
  - Line ~610-650: Speed settings and encoder config
  - Search for: "speed_preset", "rate_control", "quality_bias"

- **`ravif/src/lib.rs`**
  - Test assertions on file sizes - may give hints about expected compression

### External

- **rav1e encoder config:**
  - Check what quality/efficiency parameters ravif passes to rav1e
  - Does ravif set any `EncodeConfig` fields for quality tuning?

- **libavif equivalent:**
  - How does avifenc configure aom's encoder?
  - Any quality vs efficiency tradeoff settings?

---

## Next Steps (Prioritized)

### Phase 1: Validate Current Findings (Essential)

1. **Re-encode with SSIMULACRA2**
   - Use fast-ssim2-cli on same 5 CLIC images at Q5, Q30, Q70, Q90
   - Compare BA ratios with SSIM2 ratios
   - Confirm gap exists across multiple metrics

2. **Verify speed settings**
   - Current: speed=6 for both
   - Test: speed=4 (default for both?) to see if gap changes
   - Document what each speed actually means

3. **Check encoder defaults**
   - Confirm both using same color space, threading, etc.
   - List all non-default parameters

### Phase 2: Root Cause Analysis (Investigation)

1. **Quantizer sensitivity test**
   - Encode CLIC image at explicit qindex 50, 76, 100, 150
   - Measure BA error for each
   - Plot rav1e vs aom quality curves side-by-side

2. **Source code audit**
   - Find where rav1e speed/quality parameters set
   - Find where avifenc speed/quality parameters set
   - Identify any efficiency vs quality tuning

3. **Speed setting analysis**
   - Try speed 1, 4, 6, 10 on same image
   - See if gap narrows/widens
   - Determine if default speed choice is fair

### Phase 3: Hypothesis Testing (Deep Dive)

- If gap persists: investigate encoder tuning differences
- If gap narrows: focus on speed/preset parity
- If gap disappears: validate with SSIMULACRA2 and close issue

---

## Key Questions Remaining

1. **Why is the gap so large (30-40%) and so consistent?**
   - Is this a known difference between rav1e and aom?
   - Or a configuration/tuning difference?

2. **Why does ravif need 85% larger files at matched quality?**
   - This seems to contradict "rav1e superiority"
   - Is the original claim based on different test conditions?

3. **Should the default ravif curve be changed?**
   - Current 3-segment is aggressive (more compression)
   - But users expect Q to mean consistent quality level
   - Trade-off: compression vs semantic consistency

4. **Is libavif-scale the right answer?**
   - ✅ Makes Q semantically consistent with avifenc
   - ❌ Doesn't help ravif be more efficient
   - ✅ Enables fair benchmarking
   - Should there be a 3rd mode optimized for efficiency at matched quality?

---

## Test Data Location

```
/mnt/v/output/ravif/
├── full_quality_sweep.csv          (96 rows: 5 images × 19 Q levels)
├── QUALITY_SWEEP_ANALYSIS.md       (analysis and findings)
├── CLIC_BUTTERAUGLI_ANALYSIS.md    (earlier 3-image test)
└── FINAL_LIBAVIF_QUALITY_SUMMARY.md (API decision rationale)
```

All Butteraugli-decoded PNG files cleaned up after measurement.
Raw AVIF files still in `/tmp/full_sweep2/` if re-analysis needed.

---

## Context for Continuation

### Code State
- **Branch:** `cooperative` (15 commits ahead of r4/cooperative)
- **Recent commits:**
  - c2a27c1: Simplify with_libavif_quality() to direct libavif mapping
  - c132e47: (reverted) Attempted efficiency calibration
  - 1b3cef3: Add --libavif-scale CLI flag

### Tools Available
- `/home/lilith/work/ravif/target/debug/cavif` - CLI (compiled)
- `/home/lilith/work/aom-decode/target/release/examples/to_png` - AVIF decoder
- `/home/lilith/.local/bin/butteraugli-c` - Butteraugli measurement
- `/home/lilith/work/fast-ssim2/target/debug/fast-ssim2-cli` - SSIMULACRA2 measurement
- `/home/lilith/work/codec-corpus/clic2025-1024/` - Test corpus (5+ images)

### Build Instructions
```bash
cd /home/lilith/work/ravif
cargo build -p ravif
cargo build -p cavif
cargo test --all
cargo clippy --all-targets --all-features -- -D warnings
```

---

## Summary for Next Session

**The Issue:** Ravif's default quality curve produces significantly lower quality than avifenc at the same Q number (30-40% gap), and requires 85% larger files to match avifenc's quality.

**Current Solution:** `with_libavif_quality()` uses avifenc's linear curve, making Q semantically consistent for benchmarking.

**Open Questions:**
- Is this a fundamental encoder efficiency difference, or tuning/configuration issue?
- Should the default curve be adjusted?
- Can ravif be optimized to match or exceed avifenc's efficiency?

**How to Continue:**
1. Re-run same tests with SSIMULACRA2 metric
2. Test with different speed settings
3. Audit encoder configuration differences
4. Profile rav1e vs aom behavior at same quantizers

---

**Investigation Created By:** Claude (Haiku 4.5)
**Date:** 2026-02-07
**Estimated Context Needed:** ~20k tokens for root cause analysis
