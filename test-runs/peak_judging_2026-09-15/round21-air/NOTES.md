# How much taller should the air be? Measured, not decided

Ian's question. The answer depends on which of the **two** atmospheres in this
renderer you ask, because they do not agree with each other.

| | scale height | air at the 79km judging site | air at the 187km summit |
|---|---|---|---|
| `shared_planet.wgsl` — terrain fog, aerial perspective | 72km real | 33% | 7.5% |
| LUTs — sky, sun, skylight | 8km optical x 4.5 = **36km** | 11% | 0.6% |
| Earth, for comparison | 8.5km | 66% at Jungfraujoch | 35% at Everest |

Terrain here is **21.1x** Earth's vertical scale — a 186,709m summit against
Everest's 8,849m — so matching that asks for a scale height of **179km**. That
is **2.5x** the fog path and **5x** the LUT path.

The two being a factor of two apart is itself a finding: what lights the ground
and what you see above it are using air of different thickness. It is the likely
reason round 13 measured skylight at 0.3-0.6% of surface light while the sky
overhead still renders as an ordinary blue.

## What each setting looks like

`CATINGARDEN_AIR_SCALE_HEIGHT_KM` sets both paths to one target. Measured on the
round 19 judging views:

| | distant peak, share of the way to sky colour | ground saturation | tonal spread |
|---|---:|---:|---:|
| as shipped | 81% | 0.059 | 0.628 |
| 120km | 93% | 0.042 | 0.619 |
| 179km | 97% | 0.049 | 0.618 |

**120km is the one that looks like a photograph** (`ground-level-sweep.png`).
The distant ridge stops being a flat navy cut-out and becomes a hazy mauve
ridge with structure in it -- which also answers the "flat lavender slab with a
pink strip" all three judges named in three consecutive rounds. **179km is
milky**: the snow greys off and the near ground loses its brilliance.

So thickening the air fixes two of the four named faults at once, the missing
haze and the purple distant peaks.

## What it costs

`orbit-and-sunset-controls.png`, and this is the part that needs a decision:

- **From orbit the limb doubles in weight.** The blue halo goes from 37.2% of
  the frame to 48.0%, and it reads as a thick bright rind rather than a thin
  shell. That is a clear regression on a view that was signed off.
- **The sunset gets much warmer**, sky red-minus-blue from -2 to +73. To my eye
  that is an improvement -- the current one is nearly colourless -- but the
  sunset work was signed off as it stands, so it is not mine to call.
- `sunset_blue_hour` fails its assertions **identically** before and after, so
  that failure is pre-existing and not evidence either way. `orbit_once` passes
  in both.

## The shape of the decision

Thicker air is right at the ground and wrong at the limb, which suggests the
scale height is not the only number involved: the 2,880km gameplay shell and the
640km optical shell were sized against the present profile, and a taller
atmosphere inside the same shell is what makes the rind. Changing all three
together is a bigger piece of work than one constant.

Nothing is promoted. The knob is committed as a diagnostic, defaulting to the
constants exactly as written.

## The alternative: thicken the fog and leave the sky alone

The obvious way out is to move only the path that helps. `CATINGARDEN_AIR_SKY_UNCHANGED=1`
thickens terrain fog and aerial perspective to 179km and leaves the sky, sun and
skylight LUTs exactly as shipped. `decoupling-attempt.png`.

**It does not work, and the reason is worth keeping.**

| | distant peak toward sky | ground saturation | orbit halo |
|---|---:|---:|---:|
| as shipped | 81% | 0.059 | 37.2% |
| 120km both | 93% | 0.042 | 48.0% |
| 179km fog only | **81%** | 0.073 | **37.3%** |

The limb is preserved exactly, as intended. But the distant peak does not pale
at all: **the haze that makes distance read comes from the LUT-driven aerial
perspective, not from the terrain mist**, and that is the same path that
thickens the limb. The two cannot be separated by this knob.

Worse, from orbit the planet itself goes milky -- the terrain mist applies to
the radial column too, so the continents bleach while the limb stays thin.

So the choice is not "ground or limb". It is: accept a heavier limb, or resize
the shells the limb is drawn in, or leave the air as it is.
