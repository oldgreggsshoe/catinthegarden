# Round 13 — where the realism gap actually is, measured

**No judges were spent.** Nothing here changed the picture enough to be worth a
round, and the rule from round 9 stands: judging a no-op wastes three judges.
This is the measurement that says which direction is worth the next one.

Camera: `alpine_survey_8_directions`, HEAD (`aea0f56`), raster, 1280x720,
eight headings, median frame time **80.45ms**.

## The gap, in two numbers

The same metric on our eight captures and on the twelve Aletsch reference
photographs, taken over the lower 65% of the frame (ground, not sky):

| | ground saturation | tonal spread (p99-p01) |
|---|---:|---:|
| our render | **0.017** | **0.499** |
| Aletsch photographs | **0.176** | **0.898** |

Ground saturation is `(max-min)/max` per pixel. Ours is **ten times less
colourful** than the photographs and its ground occupies half their tonal range,
crammed into the top of it: p01 0.38, p50 0.89, p99 0.93. The references run
from p01 0.005-0.21 to p99 0.86-0.99. **We have no dark end and almost no hue.**

## Where the colour goes, stage by stage

Traced through the debug modes on heading 6 rather than guessed:

| stage | ground saturation | mean luminance |
|---|---:|---:|
| raw albedo (material decision) | 0.054 | 0.714 |
| final | 0.020 | 0.810 |

So the materials are already only a third as colourful as the photographs, and
then lighting and the tone curve remove two thirds of what is left.

## The finding: there is no fill light

Ablating the sky-diffuse term changes the frame by **at most 5 levels of 255**,
and in the lighting stage the skylight is **0.5-1.5 levels of 255** — between
**0.3% and 0.6% of surface light**. Its hue is right (blue: +1.52 against +0.65
red); its magnitude is not there.

**This is not a bug.** Computing the same integral the LUT does, on the CPU with
the shader's own constants, gives E/pi luminance **0.0949 at sea level and
0.0126 at our surface**. The scaled alpine site is 79,247m up, which the 4.5x
optical mapping puts at 17,610m against an 8,000m Rayleigh scale height: **2.2
scale heights, 11% of sea-level air overhead.** The sky there genuinely has
almost nothing to give. Real Jungfraujoch sits at 0.42 scale heights.

Two consequences follow, and they are the whole flat look:

- Every facet is lit by direct sun or by nothing, so shading is one brightness
  at one colour temperature at every orientation — which is exactly what all
  three judges reported in round 11.
- `GROUND_ALBEDO` in the LUT is 0.12. The ground here is snow at 0.64 linear.
  **Snow inter-reflection, the light an actual snowfield works by, is not
  modelled anywhere.**

Aerial perspective (2.7%) and distance mist (0.1%) were checked and are not the
cause; neither is exposure alone.

## Three candidates measured and rejected for promotion

| candidate | ground sat | spread | frame ms |
|---|---:|---:|---:|
| baseline | 0.017 | 0.499 | 80.45 |
| one-bounce ground inter-reflection | 0.018 | 0.474 | 79.04 |
| bounce + no snow greying | 0.038 | 0.474 | — |
| bounce + fixed exposure 0.50 | 0.032 | 0.559 | — |
| bounce + fixed exposure 0.35 | 0.041 | 0.563 | — |

The bounce term is physically free of tuning — a Lambertian ground of albedo `a`
under irradiance `E` gives a facet `a*E*(1-sky_view)` — and it costs nothing
measurable. **It is still a visual no-op**, and it *reduces* contrast, because it
lifts shadow without adding hue. `neutralize_snow_surface_lighting_blend` then
takes 55% of what hue there is deliberately, to stop low sun painting the icecap
orange: removing it doubles saturation, 0.018 to 0.038, and still reads the same.
Exposure moves both metrics honestly — the snow is sitting in the ACES shoulder
at 1.0 — but 0.041 against 0.176 is not a different picture.

**None of it was promoted.** `CATINGARDEN_EXPOSURE` is kept as a diagnostic so
the next exposure question is one run rather than a rebuild.

## What the eye says, attributed to code

Opening the frames (and not only the histogram) names two artefacts, each
attributed by ablating one term:

- **The smeared, waxy swirls across the foreground are the runtime detail
  ladder** (`CATINGARDEN_ABLATE=detail`, 8.05ms). With it off the snow is clean —
  and empty.
- **The leopard-spot dapple on near snow is the material detail tint**
  (`CATINGARDEN_ABLATE=tint`, 7.80ms). With it off that mottling is gone.

The two most expensive terms in the terrain shader, 15.85ms of terrain's 52.1ms,
are producing the two surface artefacts judges named in round 11. Codex reached
the same place independently from the other side in `../alpine-material-trial`:
removing both fine fragment noise and distant snow texture was ~7% faster and
did not make the snow convincing.

- **A dusty pink band sits along the distant horizon** (`evidence/pink-horizon-band-crop.png`).
  It is distant *brown lowland* seen through the distance mist: with the mist
  ablated that band is (127, 93, 65), strongly warm, and the mist carries it to
  sky blue through a magenta midpoint because green is lower than both endpoints.
  This is the "flat magenta cut-outs" judges reported.

## Recommendation

Lighting and colour are not where the remaining realism is, and this round is
the evidence for that. What is missing is **structure**: crevasses, seracs,
moraine, rock strata, a dark end to the tonal range. Two routes, and the choice
is Ian's because they trade differently against the performance rule:

1. **Cast shadows, done cheaply.** They are the only thing that creates a dark
   end, and the reference spread of 0.898 is unreachable without one. The naive
   per-pixel march was tried and dropped in `0076b4a`: +8.6ms, and judges scored
   the coarse 3km shadows *down* as "grey decals pasted on the snow". The route
   noted there — amortise into a cached texture, or bake a horizon map — is still
   open, and the cloud shadow just freed 3.76ms of budget.
2. **Spend the two expensive terms better.** 15.85ms currently buys the smear and
   the dapple. The same budget spent on coherent glacier and rock structure would
   attack the judges' first complaint instead of feeding it.

Either way the next change should be structural, and judged only once a frame
visibly differs from `evidence/baseline-006.png`.
