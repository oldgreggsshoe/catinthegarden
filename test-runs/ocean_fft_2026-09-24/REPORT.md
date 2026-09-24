# FFT wind sea: phase B plus CPU parity (24 September)

Branch `experiment/fft-ocean`. Opt-in `CATINGARDEN_OCEAN_FFT=1`; the default
renderer and CPU sea are unchanged (558 app tests pass, 0 fail, 26 ignored).

## What it is

`crates/app/src/ocean_fft.rs`, `ocean_fft.wgsl`, `ocean_fft_sample.wgsl`.
Three 256x256 Tessendorf cascades (tiles 1000m / 163m / 41m, bands
250-60m / 60-15m / <15m), directional k^-4 spectrum with cos^6(theta/2)
spreading, time quantised to a 1000s repeat. Compute: spectrum -> row IFFT ->
column IFFT -> resolve -> mips, every frame. The tiles reach the sphere through
three axis projections blended by |d.axis|^8 with variance-preserving weights.
Gerstner rows 6..18 (everything shorter than 250m) and the three normal-only
ripples are dropped; the six long swells stay Gerstner and weather-driven.
Choppy (horizontal) displacement is on, fading out between 100m and 30m depth.

## Parity

- `gpu_fft_matches_cpu_fft` (ignored, needs Vulkan; run and passes on the
  Quadro): compute-shader height grid equals a CPU radix-2 FFT of the same
  modes to 1% of sigma. Height channel only; slope channels are not pinned on
  the GPU.
- CPU queries now read cached 128x128 grids of the two long cascades
  (Catmull-Rom in space, linear in time between 0.1s snapshots, 56 kept),
  instead of summing ~1,200 modes per query. `ocean_fft_cpu_grids_match_the_mode_sum`:
  worst height 0.010m, velocity 0.008m/s, slope 0.0007, horizontal 0.006m.
  Bilinear at 64 texels was 0.47m / 0.50m/s / 0.043 / 0.48m and was rejected.
- CPU slope and vertical velocity under the FFT are centred differences of the
  reported world height, because the water over a fixed radial comes from a
  point displaced upwind: the old code returned the gradient and rate at the
  undisplaced point (analytic 0.0052 vs finite difference -0.0009 on the
  existing slope regression, with the exact mode sum too -- not a grid artefact).

## Frame cost (Quadro M1000M, 1280x720, Immediate present, DISPLAY=:0)

`ocean_deck_reference`, median frame_time_ms per run (first frame dropped),
interleaved ABBA:

| | runs | medians |
|---|---|---|
| Gerstner (off) | 7 | 38.84-39.04 |
| FFT (on) | 7 | 35.68-36.48 |

About **-2.7ms (-7%)**, every pair the same sign. `timing-final.txt`.

Before the CPU grid, FFT-on frames were ~222ms: `--profile-render` put
simulation_ms at 155 against 7.7. A backtrace on the mode-evolution cache
located the caller as `BirdFlocks::advance_over_sea`: each bird looks ahead at
up to seven instants, so queries alternated instants and summed every mode each
time. A 12-instant evolution cache moved it only to ~350ms profiled -- the sum,
not the evolution, was the cost -- which is why the grid replaced it.

## Lattice metric and appearance

`lattice_metric.py` on `ocean_manual_grid` (371m overhead): peak ring ratio
70-98 (Gerstner) -> 26-31 (FFT), median 19-23 -> 10-11. The lattice is gone.

**This is not an appearance win.** `captures/grid-fft.png` is nearly featureless
and has lost the foam; `captures/deck-fft.png` removes the lattice in the middle
distance but the foreground is as smooth as before. The plan's own gate -- "a
lattice fix that removes detail does not count" -- is not met. Measured cause:
the FFT sea is a physically moderate one (rms slope 0.088 / 0.083 / 0.090 per
cascade, ~0.15 total, CPU two-cascade 0.135 vs the Gerstner field's 0.189),
Gaussian rather than peaked, and the foam/crest terms were calibrated against
the 44x/55x Gerstner convergence, not `1 - Jacobian` at choppiness 1.1.

## Open

- Calibrate spectrum strength, choppiness and the crest/foam mask to the FFT
  sea (phase E/F work), judged at the deck view, then re-run the metric.
- FFT-mode test failures that encode Gerstner-only assumptions (8): wave-table
  size, storm height ceiling, CPU/render wave scale, depth-independent pattern
  (x2; choppiness fades with depth by design), the transport experiment (x2),
  and the waterline pose authored against the Gerstner surface.
- GPU slope channels and the far-field (phase G) are untested.
