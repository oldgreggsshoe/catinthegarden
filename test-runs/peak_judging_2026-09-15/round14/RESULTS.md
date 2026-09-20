# Round 14 — crevasses, built and not promoted

Route 2 from round 13: spend the expensive shader terms on structure rather than
on smear and dapple. **No judges spent.** The feature works, is committed, and is
**off by default** (`CATINGARDEN_CREVASSES=1`). The shipped picture is
pixel-identical to before: 0 pixels of 7,372,800 differ across the eight
headings.

## What was built

Crevasse shading that is shaded rather than painted. Two earlier attempts
(`9954f9c` era, and Codex's `experiment/glacier-fracture-material`) drew dark
lines into the albedo and were both judged as looking painted, which they were:
a line that does not move when the sun moves is a decal. This one:

- **Occludes the direct beam** inside each slot (92% at the floor) instead of
  darkening albedo. Sky fill is 0.5% of surface light here, so removing the beam
  is what actually makes a slot dark — and it gives the frame the dark end that
  round 13 measured as missing.
- **Tilts the shading normal** across each slot, so the wall turned toward the
  sun brightens and the wall away darkens. The field inverts when the sun
  crosses it. The tilt is applied to a separate `terrain_lighting_normal`, never
  to the normal the material decisions read, or a wall steep enough to shed its
  snow would turn the inside of a crevasse into rock.
- **Follows contours of the macro surface**, which is where transverse crevasses
  run, and costs nothing to know. Ground spacing then falls out of the slope for
  free: tight where the ice steepens, wide on a flat basin.
- Adds a **blue interior**: sunlight entering the ice and scattering back out,
  the one coloured shadow on this planet.

## Why it is off

| | ground saturation | tonal spread | frame ms |
|---|---:|---:|---:|
| baseline | 0.017 | 0.499 | 80.45 |
| crevasses on | 0.018 | 0.499 | 83.20 |
| Aletsch photographs | 0.176 | 0.898 | — |

0.33% of pixels move by more than 4 levels, maximum 115, so the effect is real
and local — and it reads as **faint hairlines**, not as broken ice. It is not
worth 2.75ms and it is not worth a round of judging.

**The reason is the site, not the technique.** The glacier within a kilometre of
this camera is the flat basin it stands in; the ice steep enough to crack is
three to eight kilometres away, where a metre-scale crack is sub-pixel. Two
facts fix that scale:

- The baked `crevasse_field` label is real — 3.8% of the texels on this tile —
  but its texels are 3,044m apart and **the nearest one to this camera is
  eighteen kilometres away**, so the label alone renders nothing here. The
  glacier as a whole had to become the field, with a slope band deciding which
  of it is broken.
- At 1.5km, one pixel is about 2.2m of ground. Anything that should read as
  structure at this camera has to be **tens of metres across** — icefall and
  serac scale, not crevasse scale.

Five intermediate versions were measured and are recorded in the git history of
this round. The ones worth remembering:

- Keying the contours on the *drawn* height ruled the glacier with corduroy,
  because the runtime detail ladder's own 475m wobble cycles many times over a
  few metres of ground. Macro height fixed it (`evidence/stage2-detail-contours-006.png`).
- Multiplying four sub-unit factors together left the deepest slot 15% darker
  than intact ice, which the ACES shoulder then rounded away to at most 5 levels
  of 255. Presence had to become a mask, not a weight.

## What this says about the next attempt

The technique is the right one and is now in the tree ready to use. What it
needs is a subject at the right scale: **icefall-scale broken ice, tens of metres
across**, placed where the baker says the glacier steepens, rather than
individual cracks. That is also the answer to the judges' "no crevasses, seracs
or moraines" — at this range they were never going to see a crevasse.
