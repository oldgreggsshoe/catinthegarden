# Fault 1 of 4 — the pale diagonal line is fixed

All three round 19 judges picked it out of picture 4: "a hard pale diagonal
line", "a straight pale line runs down 4's left edge", "an obvious straight
seam". It is gone. Measured on that view, the band's brightness above its local
surroundings falls **42.8 levels to 18.7**.

## Four wrong answers before the right one

Kept because each cost a build and a capture, and each would otherwise be tried
again:

1. **A LOD seam.** Rendering the same view at a 4x chunk budget put the ribbon
   in exactly the same place.
2. **The detail ladder.** Ablating it left the ribbon untouched.
3. **The material tint.** Ablating it left the ribbon untouched.
4. **The snow-lie threshold drawing a contour.** Offsetting it by the ground's
   own relief, so the snow edge would follow bumps rather than a contour,
   changed the ribbon by 0.07 of a level. That change was reverted: it is
   plausible and it is unproven.

Then a probe that returned the biome id as a pure colour channel settled it in
one run: the ribbon sits exactly on the **mountain-rock to ice border**, at the
same pixel column as the palette change, with slope and shed both flat across it.

## What it was

`terrain_material_color` replaces a snow palette with rock where the ground is
too steep to hold snow, and that replacement was weighted by
`smoothstep(0.55, 0.80)` on the *luminance of the blended palette* -- a proxy for
how much of this ground is snow.

Across an ice/rock border the proxy collapses over a narrow slice, so a strip is
left only part-replaced and the ice palette's brightness partly survives. Both
sides of the border are dark: the ice side because it sheds to rock, the rock
side because it is rock. The strip between them came out two to three times
brighter than either.

The weight is now the shed alone. Ground too steep to hold snow shows the rock
beneath whatever its palette says, and where the palette was not snow this mixes
a rock colour toward a rock colour -- which is what `rock_amount` does on the
next line anyway.

`steep_ground_sheds_its_snow_palette_without_a_luminance_proxy` fails if the
proxy comes back, mutation-checked.

## Effect elsewhere

1.9% to 15.3% of pixels change across the eight judging views, maximum 105
levels, all of it on steep ground at biome borders. 536 app and 609 workspace
tests pass; `ocean_hybrid_close` passes; `landing_site_eye_level` and
`highest_prominence_peak` fail exactly as they did before this change, both on
stale authored poses.
