# Re-siting the judging camera — method works, the planet does not have the shot

Round 17's judges all led with "no rock": the camera stands on ground that is
93.8% ice, while every reference photograph is half rock. This is the search for
a viewpoint that holds both.

## The method, which is now reliable

1. Assemble the globally dense L4 height and biome grids (6 x 2064 x 2064,
   3,044m texels) straight from the tile files.
2. Score every texel for the Jungfraujoch framing: standing on rock or snow, a
   large ice field 3-9km below within a 9km window, rock still above, no water,
   in the scaled alpine band.
3. **Constrain to sunlit ground.** The scenario's sun is fixed, so half the
   first candidate set came back black. `dot(site_direction, sun) > 0.55`.
4. **Two-pass clearance correction.** A camera placed from my own reading of the
   baked height was up to 1,236m *underground* on five of six sites -- the
   renders were dark blue slabs and slivers, and the scenario asserted pass
   throughout. Place the eye deliberately high, read `clearance_m` from the run,
   then lower it by the measured error. Second pass lands every camera within
   5m of the target.

Both corrections are worth keeping. The first cost a full cycle of confusing
pictures; the second is the only trustworthy way to author a pose here, because
the rendered surface is macro height plus a detail ladder the authoring script
cannot see.

## What was found

`pz 991,1290`, standing 3.7km above ground at 74km: **rock and ice together**,
ridges, a valley, distance and sky. Headings 1 and 2 are the best alpine views
this project has produced (`ridge-site-8-headings.png`).

**But three of its eight headings look across green and yellow lowland to brown
mesas** -- savanna, not alpine -- because the site sits on the edge of the snow
region, and two more are filled by a dark slope carrying visible diagonal seam
lines.

That is the finding: at this planet's biome resolution, a viewpoint with ice and
rock *in every direction* may not exist. The old site was consistently alpine
and had no rock; this one has rock and is not consistently alpine.

## The open question

The reference photographs are eight deliberate compositions, not a compass
sweep. Authoring the survey as eight *chosen* views at a good site would match
both the references and what a photographer does -- but it changes a protocol
Ian set, so it is his call rather than mine.

Tooling is committed: `alpine_site_probe` takes a list of candidate poses and
reports clearance for correction; `alpine_ridge_survey` is the eight-heading
survey at the site above.
