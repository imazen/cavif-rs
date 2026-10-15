# Animation metadata and premultiplication — 2026-09-07

The consumer regression in zenavif/tests/animation_encode_metadata.rs executes
real RGB/RGBA8/16 animation encodes. Before this fix, a requested Exif payload
was missing from both track and poster (encode-meta-before.log); requested
premultiplication was absent (encode-prem-before.log). Both failures reproduce
on main a7e9fcc5, whose animated.rs is unchanged from the consumer's old pin
f6c883b6. This fix is based on a7e9fcc5 and retains its zenrav1e 60594682 pin.

The animation muxer now shares configured ICC, Exif, XMP, quarter-turn rotation,
mirror, CICP, CLLI and MDCV with its color track and poster. The reported CICP
matches the current full-range BT.601 animation pixel path. Premultiplied
animation requests multiply RGB by alpha before YCbCr conversion at 8 or 10
bits and signal the matching association. Alpha itself is unchanged. Checked
serialization returns its reason through the new SerializationError variant.

The serializer dependency is the canonical published revision 98c8a501; the
consumer's local serializer manifest and nine source files match it byte for
byte. Standalone testing uses that Git dependency, without local overrides.
The upstream crate passes 86 tests/doctests with assembly disabled and 86 with
all features enabled, with five existing ignored doctests in each run. Both
all-target clippy configurations pass with warnings denied. Logs under
~/tmp/animation-metadata are upstream-animation-{tests,clippy,all-features,all-clippy}.log.

The expanded consumer regression covers all four input types at both coded
depths. It checks exact track/poster ICC/Exif/XMP, rotation/mirror and CLLI/MDCV,
plus native managed/AOM metadata. Adding container-only metadata preserves all
16 color and 8 alpha sample byte strings and exact frame timing against
controls. Three alpha levels (0,128,255), two depths and both decoders verify
straight-color output. A signaling-only mutation fails at half alpha: red 149
versus expected 75 (encode-prem-mutation.log). The mutation was restored.
Invalid rotation/mirror metadata returns a serialization error without a panic.
Final focused verification passes 3/3 (encode-meta-final-focused.log).

A full consumer nextest invocation initially placed Cargo overrides before the
external nextest subcommand; nextest did not forward them and selected the
unfixed Git package. It was stopped, and that run and its similarly invoked
clippy are not evidence for this patch. The corrected invocation puts --config
after `nextest run` / `clippy`, and logs selection of the local zenravif package.
Corrected full integration nextest passes 895/895 (nine existing skips), and
consumer all-feature library clippy passes with warnings denied. Logs are
encode-meta-nextest-corrected.log and encode-meta-clippy-corrected.log.
Libavif 1.3.0 independently decodes all 22 frames in ten exported files. Its
PNG exports retain exactly the supplied 596-byte ICC, 14-byte Exif and 37-byte
XMP at both depths. A CRC-checked PNG reader verifies all 1024 half-alpha RGBA
pixels at each depth, with channel max errors [1,1,1,0] against [75,125,175,128].
The unchanged acceptance limits are RGB <=5 and alpha <=1. See encode-meta-libavif.log
and encode-meta-reference-details.log. These reference tests provide independent
metadata-byte and rendered-premultiplication evidence, not only parser checks.

The current upstream baseline passes consumer determinism (25 legs) and 56
reference AVIF conformance cells. Its 33 byte/quality envelope rows and two
speed inversions are identical to the previous f6c883b6 baseline; the optional
armed CLI leg is unrun. No quality envelopes were repinned. Review-branch push
CI is excluded by .github/workflows/ci.yml, which targets main/master/cooperative.
No PR or workflow dispatch is part of this change.

Remaining work includes exact non-ms timing and other ignored animation coding
options, gain-map/other auxiliary metadata, broader alpha policies and video.
A read-only audit also found questionable premultiplied handling in the still
convert_alpha_8bit helper (division and endpoint handling); it is separate from
this animation path and needs an executed regression and correction next.
