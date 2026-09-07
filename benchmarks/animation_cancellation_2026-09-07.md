# Animation cancellation, 2026-09-07

Parent: cavif-rs `176ad8ee5686fcadff36d1a28478d7c61c3f9442`.

Animation ignored the legacy cancellation token, direct stop token and timeout.
The three public regressions in `ravif/tests/animation_cancellation.rs` fail
against the parent, returning a successful encoded file instead of Cancelled.
Each test submits RGB8, RGBA8, RGB16 and RGBA16 input; after correction all
format assertions pass.

One AnimationControl now spans the public call, including validation, alpha
inspection, row conversion, both tracks and pre/post serialization. The same
control is installed on every encoding context when `stop` is enabled. Its
backend status maps to Error::Cancelled. Without that feature, wrapper/row
checks still apply; per-superblock interruption requires the feature.

A deterministic backend test uses a token that fails on its twelfth check.
The one-frame wrapper performs fewer checks; sixteen superblocks cause the
failure inside the encoder. It covers 8/10-bit color and alpha. Removing only
ctx.set_stop makes this test fail, and restoring it passes. No timing/sleep
assertions or test thresholds were relaxed. Still encoding code is unchanged.

Validation: all-feature owner tests/doctests pass 90, with five existing ignored
doctests; all-target all-feature clippy passes with warnings denied. The
no-default-feature suite also passes (four existing ignored doctests).
Full logs, including the executed red and forwarding mutation, are under
`~/tmp/slower-preset-probe/animation-cancel-*.log`. Commands use the shared
run-heavy wrapper with 16G memory and four jobs.

The encoder's pre-existing quality-envelope investigation is independent and
remains open. This change does not repin quality or timing baselines.
