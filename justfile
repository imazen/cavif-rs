# Native ARM integration, with and without zenrav1e assembly.
arm-integration-audit:
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 TMPDIR="$HOME/tmp" nice -n19 cargo clippy -p zenravif --no-default-features --features threading --all-targets -- -D warnings
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 RUST_TEST_THREADS=4 TMPDIR="$HOME/tmp" nice -n19 cargo test -p zenravif
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 RUST_TEST_THREADS=4 TMPDIR="$HOME/tmp" nice -n19 cargo test -p zenravif --no-default-features --features threading
    CARGO_BUILD_JOBS=4 RAYON_NUM_THREADS=4 OMP_NUM_THREADS=4 RUST_TEST_THREADS=4 TMPDIR="$HOME/tmp" nice -n19 cargo test -p zenravif --features __expert,stop
