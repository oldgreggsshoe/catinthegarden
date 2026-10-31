# Households, children and parking — 31 October 2026

User request: "Add children, parked carts and people driving away and others
coming and parking. Perhaps give each villager their own house and their own
cart."

## What was added

**Households (`crowd.rs`).** For the 6 nearest villages (within 2.5 km) every
resident is simulated, not a handful of walkers: the 24 adult residents
(`people::resident_key`) each live in one of the village's *dwellings* (the
walk graph now marks which doors are houses, not the inn, workshop, market or
harbour), `home = dwellings[r % count]`. About half the homes have one or two
children (`people::children_in`, `child_key`). Adults go home and stay in for
40–200 s, visit public buildings and neighbours, stand about, walk to the court
mouth. Children play about the lanes: shorter outings, rarely indoors, faster.

**Children (`people.rs`/`people.wgsl`).** `Person::child_from_key`: 0.95–1.45 m,
slimmer, 1.25–1.9 m/s, no beard, apron or long coat, simpler hats; a selection
bit (`CHILD_BIT`) makes the shader draw the head 1.28x on the same skeleton;
running children lengthen the stride.

**Their own cart.** A cart's kind (open cart or covered carriage) and tint now
come from its owner's key, so the same person always has the same cart; no
resident owns two (the existing unique-driver rule covers parked carts too).

**Parking (`traffic.rs`).** Each court (from the traffic graph's village ends)
gets a ring of bays at 14 m radius, 5.4 m apart, outside the 6 m turning circle
and away from the road's approach (±120° of the far side), each checked against
the actual deck under the cart's corners (typically 10–11 per court). Agents
gained a mode: `Driving`, `Parking` (a smooth Bézier from the stop point into
the bay, driver seated), `Parked` (driver climbs down over 1.1 s, then is out),
and `Leaving` (back onto the lane 9 m past the stop). At a court stop a cart
parks in a free bay with chance 0.75 in its own village and 0.4 elsewhere, else
makes the old brief stop. Stays: 60–300 s at home, 30–150 s visiting (scaled by
`PLANET_PARKING_STAY_SCALE`). The court's existing junction zone is held while
parking and reacquired before pulling out, so it never cuts across a turning
cart. Courts within 2.5 km start with about half their bays holding residents'
carts (owners at home, departing on staggered timers). Parked carts count
separately from the moving population (cap 96; buffers sized for 160 vehicles).
Parked carts block the player like moving ones.

**Getting in and out (the two simulations talk).** Each frame the traffic
reports carts standing with their drivers out (`ParkedCart`: driver, court,
home village, door point beside the bench, time since parked, due to leave)
and the drivers who are away (on the road or on the bench). The crowd:
- hides a resident who is away, or whose cart is parked in another village;
- puts a driver who has just got down at the cart's door point, and walks
  them across the court floor to the footpath mouth and home (residents) or to
  the inn/market/a door (visitors, who stay indoors until due back);
- when a cart is due to leave, walks its driver from wherever they are to the
  mouth and across the court to the cart, and reports them "boarded";
- reports which drivers it is managing.
The traffic waits for a managed driver (up to 300 s), then plays the climb-up
and pulls out. Unmanaged drivers (villages not simulated, no footpath mouth)
board on their timer. A frame-order guard keeps a boarded driver from popping
back home if the traffic has not stepped (e.g. paused).

**Ground under people.** `TerrainRenderer::villager_ground_at`: the drawn
triangle as seen from the eye where the mesh query finds one, otherwise the
same vertex-stage height (distance-filtered detail, footpath shaping, lakes,
cuts) evaluated at the point. Measured in a replay: equal to the player-style
surface query within 1 mm for everyone within 100 m.

## Evidence (opened)

- `court_life/1793454562-45828` (15 captures, stays ×0.08): a court with
  parked carts round the ring; carts drive in and park, drivers walk off across
  the court, owners come back and drive away. 19 parked with drivers out, 2
  arrivals, 4 departures, 18 villagers in view.
- `village_children/1793454257-44957` (follows a child: a girl in skirt and
  flat cap walking the lanes), 8 children in view.
- `village_people/1793454222-44861`: adults with children about (a child next
  to a woman in a headscarf), 20 villagers / 8 children in view.
- `cart_driver_stop/1793454700-47379` and earlier replays still pass.

## Timings

Quadro M1000M, High, X11/Immediate, requested 1280x800, moon off, roads on,
`court_life` fixed court camera, stays x0.08; "off" is `PLANET_VILLAGERS=0`
(no crowd simulation or walkers; carts then board on their timers, so traffic
differs slightly). Frame-pacing p50 ms, off/on/on/off: 101.172 / 102.068 /
101.977 / 101.105, so about **+0.9 ms** for simulating ~200 villagers in six
villages plus drawing those near the court. Not FPS-neutral. The "off" runs
fail their walker assertion by design. Pacing files: `test-runs/life-2026-10-31/`.

## Tests

New: `traffic::carts_park_in_a_free_bay_wait_for_their_driver_and_pull_out_again`
(bays on the ring, clear of each other; arrive → park → driver out → recall →
waits for a managed driver, not for an unmanaged one → pulls out; no pose jump
> 0.2 m per half step), `crowd::households_fill_the_homes_children_play_and_nobody_jumps`,
`crowd::owners_leave_on_their_carts_and_visitors_come_and_go`, child cases in
`people::people_are_varied_stable_and_plausible`. The old stop test pins
parking off. Scenario registry 173. Full app suite 846 pass, same 6 pre-existing
failures, 79 ignored.

## Limits

- Bays only at road courts; harbours and villages without a court have no
  parking. Visitors whose village is not simulated just stay with their cart.
- No collision between people and carts or each other beyond keeping left and
  following; manoeuvres do not check other carts (bays are spaced apart).
- Households are fixed by index (24 adults per village however many houses);
  no families walking together, no ages beyond child/adult/older.
- On rough village ground the inspection cameras sometimes look past a crest
  so the person seems cut off; heights were checked numerically.
