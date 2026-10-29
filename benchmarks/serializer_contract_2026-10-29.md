# Serializer dependency qualification (2026-10-29)

Source: `34d2e04298be0cdd7395a3061730de98b96bc374`, based on main `ad8618d7`.
The zenavif-serialize git pin changes from `98c8a501` to
`94116d266299f347cfd10a75d440a5bc3b598d2c` to align the AVIF consumer graph.
The serializer source differs, so this is not a source-identity claim.

In an isolated export outside the workspace patches, run:

```sh
cargo metadata --format-version 1
just serializer-contract-check
```

The recipe enables `stop,imazen` without default features. All four selected
native controls passed: animation color (1), precision (2), and exact timing (1).
Strict library Clippy passed. No assertions or feature gates were weakened.
The first run enabled only `stop`: timing passed, but color/precision ran zero
tests. That coverage gap prompted the corrected recipe and complete rerun.

Measured wrapper receipt on r7900x:
`run-heavy: done rc=0 7s | peak-RSS 0.31GiB | min-avail 14879MiB | peak-load 0.37`.
This is qualification resource usage, not an encoding benchmark.

The repository has no tracked baseline Cargo.lock. The first isolated resolution
was retained unchanged for the corrected locked run; it does not establish
historical transitive dependency identity. Resolved lock SHA256: `de5ae58e35b850c3de33aff5f53d69c4b9054b698bde6cdfec80a45dec4b9c33`.

Full workspace, platform, and downstream zenavif/zenpipe checks are not covered
by this focused gate. Consumer pins and portable lock validation remain necessary.
