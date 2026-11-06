# Native ARM integration, with and without zenrav1e assembly.
arm-integration-audit:
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 TMPDIR="$HOME/tmp" nice -n19 cargo clippy -p zenravif --no-default-features --features threading --all-targets -- -D warnings
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 RUST_TEST_THREADS=4 TMPDIR="$HOME/tmp" nice -n19 cargo test -p zenravif
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 RUST_TEST_THREADS=4 TMPDIR="$HOME/tmp" nice -n19 cargo test -p zenravif --no-default-features --features threading
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 RUST_TEST_THREADS=4 TMPDIR="$HOME/tmp" nice -n19 cargo test -p zenravif --features __expert,stop

# Regenerate the public-API surface snapshots (docs/public-api/)
api-doc:
    cargo test --manifest-path apidoc/Cargo.toml

# Verify the committed snapshots are current
api-doc-check:
    ZEN_API_DOC=check cargo test --manifest-path apidoc/Cargo.toml

# Qualify the serializer used by portable consumers.
serializer-contract-check:
    cargo test -p zenravif --locked --no-default-features --features stop,imazen --test animation_timing --test animation_color --test animation_precision
    cargo clippy -p zenravif --locked --no-default-features --features stop,imazen --lib -- -D warnings

# Explicit deep-output checks, including independent ffmpeg plane equality.
cli-depth-check:
    cargo generate-lockfile
    cargo test --locked -p cavif --test source_depth
    cargo test --locked -p cavif --test source_depth oracle_ffmpeg -- --ignored
