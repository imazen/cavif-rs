# Still premultiplication correction — 2026-09-07

The public RGBA contract accepts unassociated input for every alpha mode.
`convert_alpha_8bit` instead divided RGB by alpha and replaced both opaque
and transparent pixels with transparent black. The canonical consumer's
`tests/still_premultiplication.rs` reproduced opaque alpha 0 instead of 255
before this correction. Color is now multiplied by alpha with nearest-integer
rounding; alpha is unchanged. Ordinary clean/dirty alpha paths are unchanged.
Animation already has its separate verified conversion and is unchanged here.

The consumer also corrects its raw-plane RGBA16 premultiplication and AOM
identity-color RGBA storage. Its two tests exercise both input storage types,
both 8/10 coded depths, alpha 0/128/255, and managed/AOM decoded pixels.
All twelve AVIFs independently decode in libavif 1.3.0; a CRC-checked PNG reader
checks every pixel against source RGB and alpha (invisible RGB ignored at
alpha zero). Maximum visible channel error is 3/255 and alpha is exact.
The limits remain RGB <=5/255 and alpha <=1/255. No test floor was weakened.

Owner `cargo test -p zenravif --all-features`: 86 tests/doctests pass, five
existing ignored doctests. All-feature/all-target clippy passes with warnings
denied. Consumer local-source all-feature nextest passes 897/897, nine existing
skips; default tests/doctests 484 pass, ten existing ignores; all-feature
library clippy passes. Logs are `~/tmp/animation-metadata/still-prem-*.log`.
Current quality-envelope failures require separate investigation before main
merge; this is published only to the authorized review branch. CI stays deferred.
