# FFT wind sea: phase B, CPU parity and calibration (24 September)

**Status after calibration (section at the end):** the FFT sea beats the
Gerstner sea on cost and on every detail measure -- 2.4ms faster, lattice 18-24 vs 70-98,
overhead detail 69-78 vs 51-55, deck foreground detail 32-46 vs 4-11 -- and
foam coverage is about level. The first half of this report (the flat,
foamless FFT sea) is superseded.
Human visual sign-off is still outstanding.

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

## Calibration (same day, later)

Metrics: `sea_metrics.py` -- fine contrast (std of luminance minus a 6px blur,
x1000) and foam fraction per water region, plus the lattice ratio. Captures under
xvfb; timings on DISPLAY=:0, Immediate present. Sweeps: `sweep1-5.txt`,
attributions: `attr*.txt`.

1. **Cascade gains** (`sweep1.txt`). Unit gains gave overhead fine contrast 14
   against Gerstner's 53. Lifting the short cascades adds slope, not height:
   gains 1/2/3 with choppiness 1.6 gave 88, lattice 22.
2. **Foam on the crest mask** (`sweep2.txt`, `sweep4.txt`). Slope-keyed
   whitecaps whitened whole wave faces; FFT-mode whitecaps now key off
   `1 - Jacobian` (`ocean_foam_coverage_at_crest`; underside paths keep the
   slope rule). A 0.5-0.8 ramp cut hard white sheets; 0.4-1.4 leaves
   translucent crest streaks.
3. **Damping** (`sweep3.txt`). A normal-colour diagnostic showed the deck
   foreground normals smooth while mid-distance was detailed: the 0.25m
   damping cut a 1m ripple to 30%. Deck-foreground fine contrast 15.7 (0.25m),
   31.8 (0.1m), 44.2 (0.04m). (A first run of this sweep was void: `cargo fmt`
   had reflowed the line the knob was meant to replace, and three settings
   gave byte-identical numbers.)
4. **Frame cost of the gains** (`attr.txt`, `attr3.txt`, `attr4.txt`). The
   calibrated sea first cost 40.8ms against 36.4ms at unit gains; choppiness,
   damping and foam were each neutral. Ocean submission and CPU simulation were
   unchanged; the GPU wait rose. Discarding back faces changed nothing
   (`attr2.txt`), so it is not the underside shader. It scales with the short
   cascade's gain (+3.3ms) and the middle one's (+1.4ms): near the eye the
   mesh is ~1m apart, so the short cascade roughened the near geometry. Moving
   waves under 4x the vertex spacing into shading only:
   41.0 (1x), 38.5 (2x), 36.7 (4x), 35.2 (8x) ms, Gerstner 39.1. 8x flattened
   near silhouettes and banded the horizon; **4x is the default**.

Defaults now: gains 1/2/3, choppiness 1.6, damping 0.04m, foam 0.4-1.4, vertex
filter 4x. All remain runtime knobs (`CATINGARDEN_OCEAN_FFT_TUNE`,
`_FOAM`, `_DAMPING`, `_VERTEX_FILTER`), logged as "ocean fft tune".

Final, committed defaults:

| | Gerstner | FFT calibrated |
|---|---|---|
| frame time, 7 ABBA runs | 38.56-39.35ms | 36.58-36.80ms |
| overhead fine / lattice / foam | 51-55 / 70-98 / 0.6-0.8% | 69-78 / 18-24 / 0.3-0.8% |
| deck near fine | 4-11 | 32-46 |
| deck mid fine | 24-34 | 70-108 |

`timing-calibrated.txt`; captures `captures/*-calibrated.png`. Replays passing:
ocean_manual_grid, ocean_deck_reference (2.4-2.6m clearance), ocean_rough_horizon,
ocean_hybrid_close, ocean_ship_float, ocean_steep_cusp. Tests: default 558/0/26;
FFT 7/7 including GPU parity (worst |gpu - cpu| 0.20%, 0.22% and 0.33% of the
three cascades' sigmas); FFT-mode full suite the same 8 Gerstner-assumption failures.

Still wrong or unmeasured:
- Foreground foam still reads as soft white smears on the nearest faces; it
  needs texture and temporal history (plan phase F).
- Colour is unchanged: the near water is mostly uniform sky reflection. Sea of
  Thieves' crest-lit subsurface colour is plan phase E; the crest-transmission
  anchors are still Gerstner-calibrated and now also read the FFT crest.
- The FFT band barely responds to sea state (sigma 2.2 -> 2.75m); calm and
  storm differ mostly through the Gerstner swells. Wind-driven spectrum is open.
- The CPU keeps the 15-60m cascade whole while the drawn near mesh trims ~15%
  of its shortest waves at 4x; not separately measured beyond the clearance
  assertion.
