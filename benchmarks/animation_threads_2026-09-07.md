# Animation thread forwarding — 2026-09-07

Parent: cavif-rs `2b9d4335950a952b9faa0bcd624eebf7820e714a`.
Animation built zenrav1e Config without applying Encoder::with_num_threads.
The correction forwards the requested worker count at the shared color/alpha
context constructor. Some(0) resolves the current Rayon pool size, matching
still encoding; None preserves the existing default. No tiling or coding
setting is changed.

The black-box test `ravif/tests/animation_threads.rs` observes worker pool sizes
through the live per-superblock stop callback (which never cancels). It requires
all observed coding work to use the requested pool. Before correction, the
one-thread request observes no installed worker pool and fails. Removing only
the configuration forwarding also fails the final test.

The final test encodes two frames, 65x67, in RGB8/RGBA8/RGB16/RGBA16, with
non-opaque alpha and meaningful native low bits. Requested 1, 2 and automatic
pool sizes are observed; all sixteen complete AVIFs across these settings and
None are byte-identical within each format. This is the tested geometry,
not a claim of a full large-image/thread-count sweep.

All-feature unit/integration/doc tests pass 91 (five existing ignored doctests),
no-default tests pass 58 (four existing ignored doctests), and all-target
all-feature clippy passes. RUST_TEST_THREADS=4 bounds suite concurrency as well
as the shared run-heavy wrapper's four Cargo/Rayon workers and 16G memory cap.

One initial unrestricted test run failed the existing still-image
`test_timeout_does_not_expire` five-second limit; the same test passed alone
in 0.01 seconds, and the full bounded-worker run passes. No timeout, assertion,
or test skip changed. The timeout test shares the global pool with other
encoding tests, so contention is a supported explanation, not a new measured
encoder slowdown. The changed production path is animation-only.

Raw red, forwarding mutation, green, full-suite, isolated timeout and bounded
validation logs: `~/tmp/slower-preset-probe/animation-threads-*.log`.
