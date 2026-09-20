# Round 15 — the smear was the spectrum, and it is fixed

**Promoted.** The first change this series that moves a measured number and
survives the controls. No judges spent yet; this is a foundation for the round
that does spend them.

| | ground saturation | tonal spread | frame ms |
|---|---:|---:|---:|
| before | 0.017 | **0.499** | 80.45 |
| after | 0.018 | **0.586** | 78.60 |
| Aletsch photographs | 0.176 | 0.898 | — |

The spread gap to the photographs closes by 22%, and the frame is not slower.

## Three wrong answers first

Worth recording, because each was a plausible cause and each was disproved by a
capture rather than by argument:

1. **The ridge fold.** Ridged multifractal noise makes wormy, braided ridges, so
   this was the obvious suspect. Setting `TERRAIN_DETAIL_RIDGE_STRENGTH` to zero
   left the smear untouched (`evidence/ridge-fold-off-006.png`).
2. **The direction-parameterised height field.** The code's own comment says a
   height field laid out by direction stretches down the fall line on a cliff.
   True, but the stretch is `1/cos(slope)` -- 1.4x at 45 degrees, nowhere near
   enough.
3. **Not enough geometry near the camera.** Disproved by the log: the near
   ground is already at **L18**, 128 of the 256 drawn chunks, 589,824 triangles,
   with the budget saturated.

## What it actually was

Rendering the ladder's own height as 60m contour bands
(`evidence/detail-field-60m-contours.png`) showed the near field varying over
hundreds of metres while the mid-distance slope carried dense fine structure.
The ladder's amplitude is `roughness x wavelength` at every scale -- exactly
self-similar -- so the 4m octave carried **23cm** and the 1m octave **6cm**.

That is ground that *rolls*. Shading a smooth rolling surface is what produced
the smeared, waxy look, and no amount of geometry or material work could have
fixed it, because the relief was not there to shade. Real snow and rock at those
scales are sastrugi, blocks and hollows.

## The change

A short-wavelength term in the spectral tilt: full lift below 12m, faded out by
48m, long end untouched. The long-wave gain stays at 1.0, because at 8x it made
a repeating pattern of large random basins -- that failure is still guarded.

**3.2 is not a taste setting.** It is the most lift the band can take before the
24m octave becomes louder than the 64m one, which would put a bump in an
otherwise power-law spectrum and give the ground a characteristic lump size. The
test derives that ceiling and fails above it. Gains of 5.0 and 7.0 were captured
and are visibly stronger (`evidence/short-gain-7-too-strong-006.png`) but buy
the extra contrast with exactly that artefact.

Each octave still asks `terrain_detail_octave_headroom` for room against the
ground it stands on, and the ladder's amplitude bound is re-derived from the
boosted series by its own test: 475.1m to 480.7m.

## Controls

- `alpine_survey_8_directions`, `terrain_detail_altitude_ladder` and
  `mountain_render_faults` all pass.
- `stand_on_ground` and `landing_site_eye_level` fail **identically before and
  after**, to thirteen significant figures, and those failures are stale authored
  poses that predate this work. The identical numbers are themselves a useful
  check: both cameras are kilometres up, where the distance filter has already
  retired every octave this change touches, so it provably cannot reach them.
- 536 app and 609 workspace tests, clippy and fmt pass.

## What is still missing

The lift only reaches as far as the distance filter allows, so the near field is
transformed and the mid-distance glacier is not. Saturation is untouched at
0.018 against 0.176, which round 13 already established is a separate problem
with no fill light behind it. The remaining spread gap, 0.586 against 0.898,
still needs a dark end that only cast shadows can give.
