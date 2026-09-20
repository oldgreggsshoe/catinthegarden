# Round 16 — crevasses at icefall scale: it reads, and it is still off

Round 14 built crevasse shading and shelved it because metre-scale cracks are
sub-pixel at this camera. Round 15's own finding said what to do instead: at
1.5km one pixel is 2.2m of ground, so structure has to be **tens of metres
across and hundreds apart**. Retuned to that:

| | value |
|---|---|
| elevation between cracks | 165m (a 507m ground spacing at 19 degrees) |
| slope band it appears in | 11 to 28 degrees, below where ice sheds to rock |
| broken along its length by | the ladder's own relief, already at the fragment |
| measured cost | **3.89ms**, from 6.15ms |
| ground saturation / spread | 0.021 / 0.585 against 0.018 / 0.586 with it off |

It now reads as a crevasse field on the steep ice (`evidence/icefall-scale-006.png`).
Close up it is still thinner than it should be -- more scratch than crack
(`evidence/icefall-scale-closeup.png`) -- which is the honest limit of a
fragment-stage effect on a surface with no fill light behind it.

## The cost came from a second noise lookup

Three interleaved pairs, warmed up, nothing else on the GPU: 6.15ms
(5.76/6.15/6.25). Cutting the draw range from 22km to 13km did **not** move it,
which ruled out area and pointed at per-fragment work. Half of it was a second
3D value-noise lookup -- eight corner hashes -- used only to break each crack
along its length. That now reuses `terrain_detail_meters`, the ladder's own
relief, which is already interpolated to the fragment and costs nothing. It is
also the better field: the ice surface's own bumps are what decide where it
opens. Cost fell to 3.89ms (3.89/3.78/4.17), and the picture is unchanged to the
eye.

## Why it is still off by default

3.89ms is 5% of the frame for something visible on part of it, and the standing
rule is performance first. `CATINGARDEN_CREVASSES=1` turns it on; the default
build is unchanged. Promoting it wants one of:

- a cheaper presence field than a full 3D value noise, or
- a reason to spend 5% -- most likely judges scoring it, which is a round worth
  spending once the surface it sits on has a dark end.
