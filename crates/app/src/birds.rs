//! Boids: flocking birds that also land, walk about and take off again.
//!
//! Split from `birds_render` the way `ship` is split from `ship_render`, so the
//! flocking itself stays testable without a device.
//!
//! Everything here is in the planet frame, the same f64 frame `ship` uses. A
//! bird a few hundred metres from the camera is still four thousand kilometres
//! from the planet centre, so positions never narrow to f32 until the renderer
//! has taken the camera-relative difference.

use glam::DVec3;

/// Birds are simulated on their own fixed step so the flock does not change
/// shape with frame rate. Thirty is plenty: a wingbeat is animated on the GPU
/// from a phase, not from these steps.
pub const BIRD_FIXED_STEP_SECONDS: f64 = 1.0 / 30.0;
/// Stop integrating if the caller hands us a long stall rather than fast
/// forwarding the whole backlog in one frame, exactly as the ship does.
pub const MAX_BIRD_BACKLOG_SECONDS: f64 = 0.5;

/// Flocks appear in this shell around the camera. The near edge is beyond the
/// distance at which a 0.4m bird covers a pixel at 1080p and a 60 degree field
/// of view, so a flock fades in as a speck rather than appearing in front of
/// anyone.
const FLOCK_SPAWN_MIN_METERS: f64 = 280.0;
const FLOCK_SPAWN_MAX_METERS: f64 = 460.0;
/// Hysteresis against the spawn shell: a flock is only given up once it is well
/// outside, so walking back and forth across one boundary cannot churn them.
const FLOCK_DESPAWN_METERS: f64 = 900.0;
/// How many flocks are kept *within drawing range*. Counting the whole
/// population instead was a bug worth recording: once flocks drift, all of them
/// can sit in the band between the draw distance and the despawn distance at
/// the same time. They are alive, so nothing respawns, and the sky is empty
/// while the simulation reports a full population. Topping up the near count
/// keeps birds in view and lets stragglers drift out on their own.
const MAX_NEAR_FLOCKS: usize = 6;
/// Matches `birds_render`'s draw distance. A flock outside this is simulated
/// but not drawn, so it does not count toward the near population.
const FLOCK_NEAR_RADIUS_METERS: f64 = 620.0;
/// Bounds the total including those on their way out, so the instance buffer
/// and the per-step cost both stay bounded.
const MAX_FLOCKS: usize = 10;
const FLOCK_MIN_BIRDS: usize = 9;
const FLOCK_MAX_BIRDS: usize = 26;

/// Flocks join up, but only while they are small. A flock with this many birds
/// or more neither sees other flocks nor can be seen by them, so it can neither
/// absorb nor be absorbed, and the merging stops on its own rather than running
/// away into one swarm. Both sides have to be under it, so the rule is mutual.
const FLOCK_MERGE_VISIBILITY_BIRDS: usize = 16;
/// A merge is refused outright if the result would exceed this, which is what
/// makes "no flock is ever bigger than this" a property of the code rather than
/// a hope about the thresholds. Two flocks of fifteen are the worst case.
const FLOCK_MERGE_CEILING_BIRDS: usize = 30;
/// The ceiling check in `merge_touching_flocks` and the visibility limit are
/// deliberately redundant: with these constants either one alone holds the
/// invariant, so removing the runtime check does not fail any test. What that
/// leaves unguarded is the *relationship*, which is checked here at compile
/// time instead. Raise the visibility limit without raising the ceiling and the
/// build stops, rather than the merge silently starting to be refused.
const _: () = assert!((FLOCK_MERGE_VISIBILITY_BIRDS - 1) * 2 <= FLOCK_MERGE_CEILING_BIRDS);
/// Merging has to be reachable from the spawn range, or the rule does nothing.
const _: () = assert!(FLOCK_MIN_BIRDS < FLOCK_MERGE_VISIBILITY_BIRDS);

/// Centroids inside this are close enough to be one flock.
const FLOCK_MERGE_DISTANCE_METERS: f64 = 20.0;
/// Mergeable flocks drift toward each other from here, so joining up is
/// something they do rather than something that happens to them by chance.
const FLOCK_MERGE_ATTRACTION_METERS: f64 = 160.0;
const FLOCK_MERGE_ATTRACTION_STRENGTH: f64 = 0.35;
/// Flocks that cannot merge keep out of each other's way. Without this a large
/// flock was not avoided but simply invisible, and a small flock had no reason
/// not to fly straight through it. That only looked right because encounters
/// are rare at this density: measured before the change, small and large flocks
/// came no closer than 19.77m centroid to centroid over five seeds and 300
/// seconds, entirely by luck rather than by any rule. Raise the flock cap or
/// tighten the spawn shell and it would have broken.
const FLOCK_AVOIDANCE_METERS: f64 = 90.0;
const FLOCK_AVOIDANCE_STRENGTH: f64 = 0.6;
/// A new flock is not placed on top of one already there. Steering cannot undo
/// a bad initial placement, and measurement showed that is the only thing that
/// ever mattered here: the closest two flocks came over 300 seconds was at
/// t=0.1s, on the first step after a spawn, and adding the avoidance rule above
/// moved that number by nothing at all. Pairs that could merge are still
/// allowed to start close, since those are meant to find each other.
const FLOCK_SPAWN_SEPARATION_METERS: f64 = 80.0;
/// Bearings tried before giving up on a placement this step. Without a retry a
/// crowded shell would refuse the spawn outright and let the near population
/// fall below its target.
const FLOCK_SPAWN_ATTEMPTS: usize = 8;

// Reynolds' three rules, in metres.
const NEIGHBOUR_RADIUS_METERS: f64 = 16.0;
const SEPARATION_RADIUS_METERS: f64 = 3.4;
const SEPARATION_STRENGTH: f64 = 9.0;
const ALIGNMENT_STRENGTH: f64 = 2.2;
const COHESION_STRENGTH: f64 = 1.6;
/// Holds the flock together over open ground without making it a rigid ball.
const FLOCK_ANCHOR_STRENGTH: f64 = 0.9;
const WANDER_STRENGTH: f64 = 1.4;
/// A cruising flock travels rather than holding station. Without this they sit
/// at the radius they spawned at, which at 0.42m a bird is a speck that never
/// resolves into anything. Spawn headings are biased across the camera, so
/// flocks pass by and are then given up on the far side.
const FLOCK_DRIFT_METERS_PER_SECOND: f64 = 7.5;

const CRUISE_SPEED_METERS_PER_SECOND: f64 = 11.0;
const MAX_SPEED_METERS_PER_SECOND: f64 = 17.0;
const MIN_FLYING_SPEED_METERS_PER_SECOND: f64 = 5.0;
const WALK_SPEED_METERS_PER_SECOND: f64 = 0.55;
/// Birds on the ground need their own separation. The three rules above apply
/// to birds on the wing, and a walking bird used to ignore its neighbours
/// entirely: measured, two settled birds closed to 0.014m of each other, which
/// for a 0.42m bird is one standing inside another. Flying pairs held 0.559m
/// over the same run, so only the ground case was ever wrong.
const WALK_SEPARATION_METERS: f64 = 0.75;
const WALK_SEPARATION_STRENGTH: f64 = 1.8;
/// Vertical band the flock holds while cruising, above the ground under it.
const CRUISE_ALTITUDE_MIN_METERS: f64 = 22.0;
const CRUISE_ALTITUDE_MAX_METERS: f64 = 70.0;
const ALTITUDE_HOLD_STRENGTH: f64 = 0.55;

/// Where a landing run becomes a walk, and where a walk leaves the ground.
const TOUCHDOWN_ALTITUDE_METERS: f64 = 0.35;
const TOUCHDOWN_SPEED_METERS_PER_SECOND: f64 = 2.2;
const FOOT_CLEARANCE_METERS: f64 = 0.10;
/// Ground steeper than this is not worth landing on.
const MAX_LANDING_SLOPE_RADIANS: f64 = 0.45;
/// Come near a settled flock and it leaves, which is what birds do.
const STARTLE_RADIUS_METERS: f64 = 34.0;

/// Wingbeats per second by activity. Walking birds fold their wings, so their
/// phase stops rather than slowing.
const CRUISE_WINGBEATS_PER_SECOND: f32 = 3.1;
const CLIMB_WINGBEATS_PER_SECOND: f32 = 6.4;
const GLIDE_WINGBEATS_PER_SECOND: f32 = 1.1;

/// The most birds the whole set can ever present at once: every flock at the
/// merge ceiling. This is what a renderer's instance buffer has to hold, and it
/// is larger than the spawn maximum because flocks join up.
pub const fn worst_case_bird_count() -> usize {
    MAX_FLOCKS * FLOCK_MERGE_CEILING_BIRDS
}

/// What the ground under a candidate point is like. The caller resolves this
/// from the terrain; keeping it a plain value is what lets the whole flock be
/// tested without a device or a baked tile.
#[derive(Clone, Copy, Debug)]
pub struct GroundSample {
    /// Distance from the planet centre to the ground, so nothing here needs to
    /// know the planet's radius.
    pub surface_radius_meters: f64,
    pub slope_radians: f64,
    /// False over water, ice and anything else a bird should not stand on.
    pub walkable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BirdActivity {
    Flying,
    Landing,
    Walking,
    TakingOff,
}

#[derive(Clone, Copy, Debug)]
pub struct Bird {
    /// Stable for as long as the bird exists, across merges and across the
    /// vector shuffling that `swap_remove` does. A camera following one bird
    /// needs to be able to name it; an index cannot.
    pub id: u64,
    pub position: DVec3,
    pub velocity: DVec3,
    pub activity: BirdActivity,
    /// Wingbeat cycle in turns, advanced on the CPU but applied in the vertex
    /// shader so no per-bird geometry is ever uploaded.
    pub wing_phase: f32,
    /// Where this bird was at the end of the previous fixed step, and what its
    /// wings were doing. The flock is simulated at a fixed 30Hz so a replay
    /// lands on the same birds every time, but frames are not drawn at 30Hz --
    /// so drawing the raw state shows the same pose for several frames and then
    /// jumps, which reads as a stutter and, at wingbeat rate, as the bird
    /// hopping. These let the renderer show where the bird is *between* steps.
    /// They never feed back into the simulation, so determinism is untouched.
    previous_position: DVec3,
    previous_wing_phase: f32,
    /// Per-bird offset from the flock's landing point, so a settled flock
    /// spreads over the ground instead of stacking on one spot.
    landing_offset: DVec3,
    activity_seconds: f64,
}

impl Bird {
    pub fn is_grounded(&self) -> bool {
        self.activity == BirdActivity::Walking
    }

    /// Where to draw this bird, `alpha` of the way from the previous fixed step
    /// to the current one.
    pub fn position_at(&self, alpha: f64) -> DVec3 {
        self.previous_position + (self.position - self.previous_position) * alpha
    }

    /// The wingbeat to draw, interpolated the short way round the cycle. The
    /// phase is a fraction of a turn and wraps, so a naive blend across the
    /// wrap runs the wings backwards through a whole beat in one frame.
    pub fn wing_phase_at(&self, alpha: f64) -> f32 {
        let mut delta = self.wing_phase - self.previous_wing_phase;
        if delta > 0.5 {
            delta -= 1.0;
        } else if delta < -0.5 {
            delta += 1.0;
        }
        (self.previous_wing_phase + delta * alpha as f32).rem_euclid(1.0)
    }

    /// Local up. Birds are never near the planet centre, so this is safe.
    pub fn up(&self) -> DVec3 {
        self.position.normalize()
    }

    /// The direction the bird faces. A walking bird still points along its
    /// track; a hovering one keeps its last heading rather than snapping.
    pub fn heading(&self, fallback: DVec3) -> DVec3 {
        let up = self.up();
        let flat = self.velocity - up * self.velocity.dot(up);
        if flat.length_squared() > 1.0e-6 {
            flat.normalize()
        } else {
            fallback
        }
    }
}

/// What the flock as a whole is doing. Birds land and leave together, with
/// per-bird jitter, because a flock that decided individually reads as noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlockIntent {
    Cruising,
    Settling,
    Grounded,
    Lifting,
}

#[derive(Clone, Debug)]
pub struct Flock {
    birds: Vec<Bird>,
    intent: FlockIntent,
    intent_seconds: f64,
    intent_limit_seconds: f64,
    cruise_altitude_meters: f64,
    /// Where the flock is heading while cruising, and where it is standing once
    /// settled. Held in the planet frame like everything else.
    anchor: DVec3,
    /// Tangential heading the cruising anchor travels along.
    drift: DVec3,
    wander_phase: f64,
    rng: Rng,
}

impl Flock {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn birds(&self) -> &[Bird] {
        &self.birds
    }

    pub fn centroid(&self) -> DVec3 {
        if self.birds.is_empty() {
            return self.anchor;
        }
        self.birds.iter().map(|bird| bird.position).sum::<DVec3>() / self.birds.len() as f64
    }

    /// The flock's own direction of travel: the mean of its flying birds'
    /// velocities, projected onto the local tangent plane. A per-bird heading
    /// wanders by several degrees a second, so ranking birds front-to-back
    /// against one bird's nose reshuffles the order constantly; the mean holds
    /// still enough to mean "the back of the flock".
    fn mean_heading(&self) -> Option<DVec3> {
        let up = self.centroid().normalize();
        let velocity: DVec3 = self
            .birds
            .iter()
            .filter(|bird| !bird.is_grounded())
            .map(|bird| bird.velocity)
            .sum();
        let flat = velocity - up * velocity.dot(up);
        (flat.length() > 1e-6).then(|| flat.normalize())
    }

    /// A bird toward the back of the flock, which is the one worth riding: a
    /// camera sitting behind it has the whole flock ahead of the lens.
    ///
    /// Two properties are wanted and they are not the same bird. Being at the
    /// back puts the flock in front; being near the flock's own axis puts it
    /// straight ahead rather than off to one side. So the rear third is taken
    /// first, on the flock's mean heading, and the most central of those is
    /// ridden. The very rearmost bird alone is a worse shot often enough to be
    /// worth the extra step, because it is frequently the one that has fallen
    /// out to a flank.
    fn rearward_bird(&self) -> Option<&Bird> {
        let centroid = self.centroid();
        let forward = self.mean_heading()?;
        let mut flying: Vec<&Bird> = self
            .birds
            .iter()
            .filter(|bird| !bird.is_grounded())
            .collect();
        if flying.is_empty() {
            return None;
        }
        flying.sort_by(|left, right| {
            let along = |bird: &Bird| (bird.position - centroid).dot(forward);
            along(left)
                .total_cmp(&along(right))
                .then_with(|| left.id.cmp(&right.id))
        });
        // At least one, so a flock of one or two still has a rider.
        let rear = flying.len().div_ceil(3).max(1);
        flying[..rear]
            .iter()
            .min_by(|left, right| {
                let lateral = |bird: &Bird| {
                    let offset = bird.position - centroid;
                    (offset - forward * offset.dot(forward)).length()
                };
                lateral(left)
                    .total_cmp(&lateral(right))
                    .then_with(|| left.id.cmp(&right.id))
            })
            .copied()
    }

    /// Small enough, and settled enough, to take an interest in other flocks.
    /// Birds on the ground or on their way to it are busy.
    fn is_mergeable(&self) -> bool {
        self.intent == FlockIntent::Cruising && self.birds.len() < FLOCK_MERGE_VISIBILITY_BIRDS
    }

    fn grounded_count(&self) -> usize {
        self.birds.iter().filter(|bird| bird.is_grounded()).count()
    }
}

/// Deterministic splitmix64. There is no `rand` in this workspace and scenario
/// replays have to land on the same flock every time, so the generator is part
/// of the simulation rather than the environment.
#[derive(Clone, Copy, Debug)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Half-open [0, 1), from the top 53 bits so the low-order weakness of a
    /// truncated multiply cannot show up as banding in a direction.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    fn index(&mut self, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        (self.next_u64() % count as u64) as usize
    }

    /// Uniform on the sphere. Rejection would also work but this is branchless
    /// and the distribution matters for spawn rings.
    fn unit_vector(&mut self) -> DVec3 {
        let z = self.range(-1.0, 1.0);
        let angle = self.range(0.0, std::f64::consts::TAU);
        let radius = (1.0 - z * z).max(0.0).sqrt();
        DVec3::new(radius * angle.cos(), radius * angle.sin(), z)
    }
}

/// Two unit vectors spanning the plane perpendicular to `up`.
fn tangent_basis(up: DVec3) -> (DVec3, DVec3) {
    let reference = if up.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let east = up.cross(reference).normalize();
    (east, up.cross(east))
}

fn limit(vector: DVec3, maximum: f64) -> DVec3 {
    let length_squared = vector.length_squared();
    if length_squared > maximum * maximum && length_squared > 0.0 {
        vector * (maximum / length_squared.sqrt())
    } else {
        vector
    }
}

pub struct BirdFlocks {
    flocks: Vec<Flock>,
    rng: Rng,
    /// Simulation clock, carried so a caller can hand us wall time and let the
    /// fixed step do the accounting.
    simulated_seconds: f64,
    /// See `interpolation_alpha`. Render-only.
    interpolation_alpha: f64,
    /// Merges so far. Exposed because "flocks join up" is otherwise invisible
    /// in a replay: the bird count does not change and the flock count falls
    /// the same way a retirement makes it fall.
    merges: u64,
    next_bird_id: u64,
    /// The shell new flocks appear in. A field rather than a constant only so a
    /// demo or a diagnostic replay can bring it in close and show birds at
    /// arm's length without waiting for one to drift over; the shipping values
    /// are the constants above. Reading the environment is left to the caller,
    /// which is what keeps this module testable without one.
    spawn_min_meters: f64,
    spawn_max_meters: f64,
}

impl BirdFlocks {
    pub fn new(seed: u64) -> Self {
        Self {
            flocks: Vec::new(),
            rng: Rng::new(seed),
            simulated_seconds: 0.0,
            interpolation_alpha: 0.0,
            merges: 0,
            next_bird_id: 1,
            spawn_min_meters: FLOCK_SPAWN_MIN_METERS,
            spawn_max_meters: FLOCK_SPAWN_MAX_METERS,
        }
    }

    /// Overrides the spawn shell. Ignored unless both bounds are finite,
    /// positive and ordered, so a malformed override cannot quietly produce a
    /// flock inside the camera.
    pub fn with_spawn_shell(mut self, min_meters: f64, max_meters: f64) -> Self {
        if min_meters.is_finite()
            && max_meters.is_finite()
            && min_meters > 0.0
            && max_meters > min_meters
        {
            self.spawn_min_meters = min_meters;
            self.spawn_max_meters = max_meters;
        }
        self
    }

    /// Applies `CATINGARDEN_BIRD_SPAWN_METERS`, written as `min,max` in metres.
    /// Anything unparseable leaves the shipping shell alone.
    ///
    /// `allow(dead_code)` because the only caller is the binary's own
    /// construction of the flock set: a build that does not link it has no use
    /// for the hook, and the alternative is a warning that says nothing.
    #[allow(dead_code)]
    pub fn with_spawn_shell_from_env(self) -> Self {
        let Ok(value) = std::env::var("CATINGARDEN_BIRD_SPAWN_METERS") else {
            return self;
        };
        let Some((min, max)) = value.split_once(',') else {
            return self;
        };
        match (min.trim().parse::<f64>(), max.trim().parse::<f64>()) {
            (Ok(min), Ok(max)) => self.with_spawn_shell(min, max),
            _ => self,
        }
    }

    pub fn bird_count(&self) -> usize {
        self.flocks.iter().map(|flock| flock.birds.len()).sum()
    }

    pub fn flock_count(&self) -> usize {
        self.flocks.len()
    }

    pub fn merge_count(&self) -> u64 {
        self.merges
    }

    /// Largest flock currently alive, which is the number the merge ceiling
    /// exists to bound.
    pub fn largest_flock(&self) -> usize {
        self.flocks
            .iter()
            .map(|flock| flock.birds.len())
            .max()
            .unwrap_or(0)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn flocks(&self) -> &[Flock] {
        &self.flocks
    }

    /// The bird a camera should ride, given where the rider is standing and the
    /// bird it is riding now.
    ///
    /// Keeps `current` while that bird exists and only chooses again once it is
    /// gone: stability is the point, since re-picking every frame would cut
    /// between birds continuously. A fresh pick takes the nearest flock that
    /// has anything airborne in it -- nearest, because this is reached by
    /// pressing a key while looking at a flock, and the flock being looked at
    /// is the one meant -- and rides one of its rearward birds, so the flock is
    /// ahead of the lens.
    pub fn ride_target(&self, from: DVec3, current: Option<u64>) -> Option<u64> {
        if let Some(current) = current
            && self.bird(current).is_some()
        {
            return Some(current);
        }
        self.flocks
            .iter()
            .filter(|flock| flock.birds.iter().any(|bird| !bird.is_grounded()))
            .min_by(|left, right| {
                let range = |flock: &Flock| (flock.centroid() - from).length_squared();
                range(left).total_cmp(&range(right))
            })
            .and_then(Flock::rearward_bird)
            .map(|bird| bird.id)
    }

    /// Eye, aim and up for a camera riding `target`, in the planet frame.
    ///
    /// Aims at the flock rather than straight down the bird's nose. Riding a
    /// bird and looking along its heading frames an empty sky whenever that
    /// bird happens to be out in front, and the flockmates are most of what
    /// makes the shot worth watching. The aim is pulled back toward the bird
    /// when the flock is nearly on top of it, so a tight flock does not swing
    /// the camera around.
    pub fn chase_camera(
        &self,
        target: u64,
        behind_meters: f64,
        above_meters: f64,
        ahead_meters: f64,
    ) -> Option<(DVec3, DVec3, DVec3)> {
        let flock = self
            .flocks
            .iter()
            .find(|flock| flock.birds.iter().any(|bird| bird.id == target))?;
        let bird = flock.birds.iter().find(|bird| bird.id == target)?;
        // The same interpolated pose the renderer draws. Riding the raw stepped
        // position puts the camera on a 30Hz staircase while every bird around
        // it moves smoothly, which is worse than either on its own.
        let position = bird.position_at(self.interpolation_alpha);
        let up = position.normalize();
        let forward = bird.heading(tangent_basis(up).0);
        let eye = position - forward * behind_meters + up * above_meters;

        // Aim at the flock, but never behind the bird. Aiming straight at the
        // centroid swings the camera round to look back down the flock whenever
        // the bird's own nose has turned away from the middle, which it does
        // constantly -- measured, that framed well in only 57.5% of frames.
        // Pushing the aim forward to a minimum standoff keeps the lateral pull
        // toward the flock, so flockmates stay in shot, while the view still
        // points the way the bird is going.
        let mut aim = flock.centroid();
        let minimum_ahead = ahead_meters * 0.5;
        let forward_component = (aim - position).dot(forward);
        if forward_component < minimum_ahead {
            aim += forward * (minimum_ahead - forward_component);
        }
        Some((eye, aim, up))
    }

    /// Every flock's centre, in the planet frame. The caller picks which one to
    /// mark, because "nearest" alone is the wrong question: the closest flock
    /// is frequently behind the camera, and a marker on something behind you
    /// points at nothing. Only the caller has the view basis to tell.
    pub fn flock_centroids(&self) -> impl Iterator<Item = DVec3> {
        self.flocks.iter().map(|flock| flock.centroid())
    }

    pub fn bird(&self, id: u64) -> Option<&Bird> {
        self.birds().find(|bird| bird.id == id)
    }

    pub fn birds(&self) -> impl Iterator<Item = &Bird> {
        self.flocks.iter().flat_map(|flock| flock.birds.iter())
    }

    /// Advance to `target_seconds`, spawning and retiring flocks around
    /// `camera_local`. `ground` resolves the surface under a direction and may
    /// decline, which is how unloaded terrain and unwalkable ground are both
    /// handled: a flock that cannot find ground simply stays airborne.
    pub fn advance(
        &mut self,
        target_seconds: f64,
        camera_local: DVec3,
        ground: &dyn Fn(DVec3) -> Option<GroundSample>,
    ) {
        if target_seconds < self.simulated_seconds {
            // A scenario replay can rewind the clock; restart rather than spin.
            self.simulated_seconds = target_seconds;
            return;
        }
        if target_seconds - self.simulated_seconds > MAX_BIRD_BACKLOG_SECONDS {
            self.simulated_seconds = target_seconds - MAX_BIRD_BACKLOG_SECONDS;
        }
        while self.simulated_seconds + BIRD_FIXED_STEP_SECONDS <= target_seconds {
            self.simulated_seconds += BIRD_FIXED_STEP_SECONDS;
            for flock in &mut self.flocks {
                for bird in &mut flock.birds {
                    bird.previous_position = bird.position;
                    bird.previous_wing_phase = bird.wing_phase;
                }
            }
            self.step(BIRD_FIXED_STEP_SECONDS, camera_local, ground);
        }
        // Whatever time is left over is how far into the next step the frame
        // being drawn sits.
        self.interpolation_alpha =
            ((target_seconds - self.simulated_seconds) / BIRD_FIXED_STEP_SECONDS).clamp(0.0, 1.0);
    }

    /// How far the frame about to be drawn sits between the last completed
    /// fixed step and the next one, in 0..=1. Render-only: nothing in the
    /// simulation reads it, so replays stay deterministic.
    pub fn interpolation_alpha(&self) -> f64 {
        self.interpolation_alpha
    }

    fn step(
        &mut self,
        step_seconds: f64,
        camera_local: DVec3,
        ground: &dyn Fn(DVec3) -> Option<GroundSample>,
    ) {
        // Move first, then settle the population. The renderer reads this
        // state after the step, so topping up last is what makes "there are
        // birds within drawing range" true of the frame that gets drawn rather
        // than of an instant in the middle of it.
        for flock in &mut self.flocks {
            advance_flock(flock, step_seconds, camera_local, ground);
        }
        self.steer_flocks_past_each_other(step_seconds);
        self.merge_touching_flocks();
        self.retire_distant_flocks(camera_local);
        self.spawn_missing_flocks(camera_local, ground);
    }

    /// Steers each travelling flock against the others. A pair that could merge
    /// is drawn together; every other pair pushes apart. There is deliberately
    /// no third case: leaving non-mergeable flocks with no interaction at all
    /// is what let a small flock fly through a large one.
    ///
    /// Computed against a snapshot of centroids so the pass does not depend on
    /// the order flocks happen to sit in the vector.
    fn steer_flocks_past_each_other(&mut self, step_seconds: f64) {
        let summary: Vec<(DVec3, usize, bool)> = self
            .flocks
            .iter()
            .map(|flock| (flock.centroid(), flock.birds.len(), flock.is_mergeable()))
            .collect();
        for (index, flock) in self.flocks.iter_mut().enumerate() {
            // Only a flock whose anchor actually travels can be steered; a
            // settled one is standing on its landing ground.
            if !matches!(flock.intent, FlockIntent::Cruising | FlockIntent::Lifting) {
                continue;
            }
            let (centroid, count, mergeable) = summary[index];
            let up = flock.anchor.normalize();
            let mut steer = DVec3::ZERO;
            let mut nearest_partner: Option<(f64, DVec3)> = None;

            for (other_index, (other_centroid, other_count, other_mergeable)) in
                summary.iter().enumerate()
            {
                if other_index == index {
                    continue;
                }
                let distance = centroid.distance(*other_centroid);
                let could_merge = mergeable
                    && *other_mergeable
                    && count + other_count <= FLOCK_MERGE_CEILING_BIRDS;
                if could_merge {
                    if distance <= FLOCK_MERGE_ATTRACTION_METERS
                        && nearest_partner.is_none_or(|(best, _)| distance < best)
                    {
                        nearest_partner = Some((distance, *other_centroid));
                    }
                    continue;
                }
                if distance > FLOCK_AVOIDANCE_METERS || distance <= 1.0e-6 {
                    continue;
                }
                let away = centroid - *other_centroid;
                let flat = away - up * away.dot(up);
                if flat.length_squared() <= 1.0e-9 {
                    continue;
                }
                // Linear in how far inside the radius the other flock is, so a
                // distant one barely registers and a close one is firm.
                steer += flat.normalize()
                    * (((FLOCK_AVOIDANCE_METERS - distance) / FLOCK_AVOIDANCE_METERS)
                        * FLOCK_AVOIDANCE_STRENGTH);
            }

            if let Some((_, target)) = nearest_partner {
                let toward = target - flock.anchor;
                let flat = toward - up * toward.dot(up);
                if flat.length_squared() > 1.0e-9 {
                    steer += flat.normalize() * FLOCK_MERGE_ATTRACTION_STRENGTH;
                }
            }

            if steer.length_squared() <= 1.0e-12 {
                continue;
            }
            let blended = flock.drift + steer * (step_seconds * 10.0);
            if blended.length_squared() > 1.0e-9 {
                flock.drift = blended.normalize();
            }
        }
    }

    /// Joins one touching pair per step. One at a time keeps the bookkeeping
    /// obvious and cannot cascade several flocks into a swarm inside a single
    /// step, which is the failure this whole rule exists to prevent.
    fn merge_touching_flocks(&mut self) {
        let mut pair = None;
        'outer: for left in 0..self.flocks.len() {
            for right in (left + 1)..self.flocks.len() {
                if !self.flocks[left].is_mergeable() || !self.flocks[right].is_mergeable() {
                    continue;
                }
                if self.flocks[left].birds.len() + self.flocks[right].birds.len()
                    > FLOCK_MERGE_CEILING_BIRDS
                {
                    continue;
                }
                if self.flocks[left]
                    .centroid()
                    .distance(self.flocks[right].centroid())
                    <= FLOCK_MERGE_DISTANCE_METERS
                {
                    pair = Some((left, right));
                    break 'outer;
                }
            }
        }
        let Some((left, right)) = pair else {
            return;
        };
        // The larger flock keeps its own heading and landing plans; the smaller
        // one joins it. Equal sizes fall to the earlier index, which is stable.
        let (keep, absorb) = if self.flocks[left].birds.len() >= self.flocks[right].birds.len() {
            (left, right)
        } else {
            (right, left)
        };
        let joining = self.flocks.swap_remove(absorb);
        // `swap_remove` moves the last element into `absorb`, so the survivor's
        // index only shifts if it was that last element.
        let keep = if keep == self.flocks.len() {
            absorb
        } else {
            keep
        };
        self.flocks[keep].birds.extend(joining.birds);
        self.merges += 1;
        debug_assert!(self.flocks[keep].birds.len() <= FLOCK_MERGE_CEILING_BIRDS);
    }

    fn retire_distant_flocks(&mut self, camera_local: DVec3) {
        self.flocks
            .retain(|flock| flock.centroid().distance(camera_local) <= FLOCK_DESPAWN_METERS);
    }

    fn near_flock_count(&self, camera_local: DVec3) -> usize {
        self.flocks
            .iter()
            .filter(|flock| flock.centroid().distance(camera_local) <= FLOCK_NEAR_RADIUS_METERS)
            .count()
    }

    fn spawn_missing_flocks(
        &mut self,
        camera_local: DVec3,
        ground: &dyn Fn(DVec3) -> Option<GroundSample>,
    ) {
        while self.near_flock_count(camera_local) < MAX_NEAR_FLOCKS {
            if self.flocks.len() >= MAX_FLOCKS {
                // Outbound stragglers had filled the cap and blocked the near
                // top-up, which is the same empty sky by another route. Give up
                // the furthest instead; by construction it is already outside
                // the near radius and on its way out anyway.
                let Some((furthest, _)) = self
                    .flocks
                    .iter()
                    .enumerate()
                    .map(|(index, flock)| (index, flock.centroid().distance(camera_local)))
                    .max_by(|left, right| left.1.total_cmp(&right.1))
                else {
                    return;
                };
                self.flocks.swap_remove(furthest);
            }
            let Some(flock) = self.spawn_flock(camera_local, ground) else {
                // No walkable ground in reach this step -- over open ocean, or
                // terrain that has not streamed in yet. Try again next step
                // rather than burning the whole budget on one frame.
                return;
            };
            self.flocks.push(flock);
        }
    }

    fn spawn_flock(
        &mut self,
        camera_local: DVec3,
        ground: &dyn Fn(DVec3) -> Option<GroundSample>,
    ) -> Option<Flock> {
        let up = camera_local.normalize();
        let (east, north) = tangent_basis(up);
        let mut placement = None;
        for _ in 0..FLOCK_SPAWN_ATTEMPTS {
            let bearing = self.rng.range(0.0, std::f64::consts::TAU);
            let distance = self.rng.range(self.spawn_min_meters, self.spawn_max_meters);
            let offset = east * (distance * bearing.cos()) + north * (distance * bearing.sin());
            let anchor_direction = (camera_local + offset).normalize();
            // Resolvable ground is required, because the flock needs to know
            // how high it is. Walkable ground is not: birds fly over water all
            // the time, they just do not land on it. Requiring it here meant no
            // flock could exist over the sea or near a waterline at all, which
            // is how a demo camera on a beach came back with an empty sky.
            let Some(sample) = ground(anchor_direction) else {
                continue;
            };
            let cruise_altitude_meters = self
                .rng
                .range(CRUISE_ALTITUDE_MIN_METERS, CRUISE_ALTITUDE_MAX_METERS);
            let anchor = anchor_direction * (sample.surface_radius_meters + cruise_altitude_meters);
            // Crowding a flock this one could merge with is fine; crowding any
            // other is what put two flocks 19.77m apart on their first step.
            let crowded = self.flocks.iter().any(|flock| {
                flock.centroid().distance(anchor) < FLOCK_SPAWN_SEPARATION_METERS
                    && !flock.is_mergeable()
            });
            if crowded {
                continue;
            }
            placement = Some((anchor_direction, anchor, cruise_altitude_meters));
            break;
        }
        let (anchor_direction, anchor, cruise_altitude_meters) = placement?;
        let count = FLOCK_MIN_BIRDS + self.rng.index(FLOCK_MAX_BIRDS - FLOCK_MIN_BIRDS + 1);
        let mut rng = Rng::new(self.rng.next_u64());
        let (flock_east, flock_north) = tangent_basis(anchor_direction);
        let heading =
            flock_east * self.rng.range(-1.0, 1.0) + flock_north * self.rng.range(-1.0, 1.0);
        let heading = if heading.length_squared() > 1.0e-9 {
            heading.normalize()
        } else {
            flock_east
        };

        let mut birds = Vec::with_capacity(count);
        for _ in 0..count {
            let scatter = flock_east * rng.range(-9.0, 9.0)
                + flock_north * rng.range(-9.0, 9.0)
                + anchor_direction * rng.range(-4.0, 4.0);
            let id = self.next_bird_id;
            self.next_bird_id += 1;
            let position = anchor + scatter;
            // Drawn in the order the fields used to be written in. Hoisting a
            // `let` out of a struct literal moves where its generator call
            // happens, and this generator *is* the simulation: reordering these
            // two silently reseeds every bird's velocity and wingbeat, which
            // showed up as two unrelated flocking tests failing.
            let velocity =
                heading * CRUISE_SPEED_METERS_PER_SECOND + rng.unit_vector() * rng.range(0.0, 1.5);
            let wing_phase = rng.unit() as f32;
            birds.push(Bird {
                id,
                position,
                velocity,
                activity: BirdActivity::Flying,
                wing_phase,
                // A new bird has no previous step, so it starts standing still
                // rather than being interpolated in from the planet centre.
                previous_position: position,
                previous_wing_phase: wing_phase,
                landing_offset: flock_east * rng.range(-7.0, 7.0)
                    + flock_north * rng.range(-7.0, 7.0),
                activity_seconds: 0.0,
            });
        }

        // Head generally toward the camera, turned by up to about 75 degrees,
        // so flocks cross the view instead of either ignoring it or homing in.
        let toward_camera = camera_local - anchor;
        let flat = toward_camera - anchor_direction * toward_camera.dot(anchor_direction);
        let base = if flat.length_squared() > 1.0e-9 {
            flat.normalize()
        } else {
            flock_east
        };
        let turn = rng.range(-1.3, 1.3);
        let drift = base * turn.cos() + anchor_direction.cross(base) * turn.sin();

        Some(Flock {
            birds,
            intent: FlockIntent::Cruising,
            intent_seconds: 0.0,
            intent_limit_seconds: rng.range(14.0, 40.0),
            cruise_altitude_meters,
            anchor,
            drift,
            wander_phase: rng.range(0.0, std::f64::consts::TAU),
            rng,
        })
    }
}

fn advance_flock(
    flock: &mut Flock,
    step_seconds: f64,
    camera_local: DVec3,
    ground: &dyn Fn(DVec3) -> Option<GroundSample>,
) {
    flock.intent_seconds += step_seconds;
    flock.wander_phase += step_seconds * 0.6;
    update_intent(flock, camera_local, ground);
    advance_anchor(flock, step_seconds, ground);

    let centroid = flock.centroid();
    let average_velocity = if flock.birds.is_empty() {
        DVec3::ZERO
    } else {
        flock.birds.iter().map(|bird| bird.velocity).sum::<DVec3>() / flock.birds.len() as f64
    };

    // Snapshot positions so every bird steers against the same instant rather
    // than against half-updated neighbours, which biases the flock toward
    // whichever bird happens to be first in the vector.
    let snapshot: Vec<(DVec3, DVec3)> = flock
        .birds
        .iter()
        .map(|bird| (bird.position, bird.velocity))
        .collect();

    let intent = flock.intent;
    let cruise_altitude_meters = flock.cruise_altitude_meters;
    let anchor = flock.anchor;
    let wander_phase = flock.wander_phase;

    for (index, bird) in flock.birds.iter_mut().enumerate() {
        bird.activity_seconds += step_seconds;
        let up = bird.up();
        let sample = ground(bird.position.normalize());
        let surface_radius = sample.map(|sample| sample.surface_radius_meters);

        match bird.activity {
            BirdActivity::Walking => {
                let separation = walking_separation(bird, index, &snapshot, up);
                step_walking_bird(bird, step_seconds, surface_radius, up, intent, separation);
            }
            _ => {
                let steering = flying_steering(
                    bird,
                    index,
                    &snapshot,
                    centroid,
                    average_velocity,
                    anchor,
                    intent,
                    cruise_altitude_meters,
                    surface_radius,
                    up,
                    wander_phase,
                );
                step_flying_bird(bird, step_seconds, steering, up, surface_radius, intent);
            }
        }
    }
}

/// Carries a cruising flock's anchor along its heading and keeps it at the
/// cruise altitude over whatever ground it is now above. A settled or settling
/// flock's anchor is its landing ground, so it stays put.
fn advance_anchor(
    flock: &mut Flock,
    step_seconds: f64,
    ground: &dyn Fn(DVec3) -> Option<GroundSample>,
) {
    if !matches!(flock.intent, FlockIntent::Cruising | FlockIntent::Lifting) {
        return;
    }
    let up = flock.anchor.normalize();
    // Re-flatten every step: travelling over a sphere tips the heading out of
    // the tangent plane, and left alone that walks the anchor off the surface.
    let flat = flock.drift - up * flock.drift.dot(up);
    if flat.length_squared() > 1.0e-9 {
        flock.drift = flat.normalize();
        flock.anchor += flock.drift * (FLOCK_DRIFT_METERS_PER_SECOND * step_seconds);
    }
    if let Some(sample) = ground(flock.anchor.normalize()) {
        flock.anchor = flock.anchor.normalize()
            * (sample.surface_radius_meters + flock.cruise_altitude_meters);
    }
}

/// Flock-level decisions: when to come down, how long to stay, when to leave.
fn update_intent(
    flock: &mut Flock,
    camera_local: DVec3,
    ground: &dyn Fn(DVec3) -> Option<GroundSample>,
) {
    let centroid = flock.centroid();
    let startled = centroid.distance(camera_local) < STARTLE_RADIUS_METERS;

    match flock.intent {
        FlockIntent::Cruising => {
            if flock.intent_seconds >= flock.intent_limit_seconds && !startled {
                // Only commit to a landing if there is somewhere to land.
                let direction = centroid.normalize();
                if let Some(sample) = ground(direction)
                    && sample.walkable
                    && sample.slope_radians <= MAX_LANDING_SLOPE_RADIANS
                {
                    flock.anchor = direction * sample.surface_radius_meters;
                    flock.intent = FlockIntent::Settling;
                    flock.intent_seconds = 0.0;
                    flock.intent_limit_seconds = flock.rng.range(8.0, 16.0);
                    for bird in &mut flock.birds {
                        if bird.activity == BirdActivity::Flying {
                            bird.activity = BirdActivity::Landing;
                            bird.activity_seconds = 0.0;
                        }
                    }
                }
                if flock.intent == FlockIntent::Cruising {
                    // Nowhere to put down; look again shortly.
                    flock.intent_seconds = 0.0;
                    flock.intent_limit_seconds = flock.rng.range(6.0, 14.0);
                }
            }
        }
        FlockIntent::Settling => {
            let grounded = flock.grounded_count();
            if startled || flock.intent_seconds >= flock.intent_limit_seconds * 2.0 {
                begin_lifting(flock);
            } else if grounded * 2 >= flock.birds.len().max(1) {
                flock.intent = FlockIntent::Grounded;
                flock.intent_seconds = 0.0;
                flock.intent_limit_seconds = flock.rng.range(10.0, 30.0);
            }
        }
        FlockIntent::Grounded => {
            if startled || flock.intent_seconds >= flock.intent_limit_seconds {
                begin_lifting(flock);
            }
        }
        FlockIntent::Lifting => {
            if flock.intent_seconds >= 2.5 {
                flock.intent = FlockIntent::Cruising;
                flock.intent_seconds = 0.0;
                flock.intent_limit_seconds = flock.rng.range(18.0, 45.0);
                flock.cruise_altitude_meters = flock
                    .rng
                    .range(CRUISE_ALTITUDE_MIN_METERS, CRUISE_ALTITUDE_MAX_METERS);
                for bird in &mut flock.birds {
                    if bird.activity == BirdActivity::TakingOff {
                        bird.activity = BirdActivity::Flying;
                        bird.activity_seconds = 0.0;
                    }
                }
            }
        }
    }
}

fn begin_lifting(flock: &mut Flock) {
    flock.intent = FlockIntent::Lifting;
    flock.intent_seconds = 0.0;
    for bird in &mut flock.birds {
        if bird.activity != BirdActivity::Flying {
            bird.activity = BirdActivity::TakingOff;
            bird.activity_seconds = 0.0;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn flying_steering(
    bird: &Bird,
    index: usize,
    snapshot: &[(DVec3, DVec3)],
    centroid: DVec3,
    average_velocity: DVec3,
    anchor: DVec3,
    intent: FlockIntent,
    cruise_altitude_meters: f64,
    surface_radius: Option<f64>,
    up: DVec3,
    wander_phase: f64,
) -> DVec3 {
    let mut separation = DVec3::ZERO;
    let mut alignment = DVec3::ZERO;
    let mut cohesion = DVec3::ZERO;
    let mut neighbours = 0.0;

    for (other_index, (position, velocity)) in snapshot.iter().enumerate() {
        if other_index == index {
            continue;
        }
        let offset = bird.position - *position;
        let distance_squared = offset.length_squared();
        if distance_squared > NEIGHBOUR_RADIUS_METERS * NEIGHBOUR_RADIUS_METERS {
            continue;
        }
        neighbours += 1.0;
        alignment += *velocity;
        cohesion += *position;
        if distance_squared < SEPARATION_RADIUS_METERS * SEPARATION_RADIUS_METERS
            && distance_squared > 1.0e-6
        {
            // Inverse distance, so a near miss pushes far harder than a
            // neighbour at the edge of the separation radius.
            separation += offset / distance_squared;
        }
    }

    let mut steering = separation * SEPARATION_STRENGTH;
    if neighbours > 0.0 {
        steering +=
            (alignment / neighbours - bird.velocity).normalize_or_zero() * ALIGNMENT_STRENGTH;
        steering += (cohesion / neighbours - bird.position).normalize_or_zero() * COHESION_STRENGTH;
    } else {
        steering += (centroid - bird.position).normalize_or_zero() * COHESION_STRENGTH;
        steering += average_velocity.normalize_or_zero() * ALIGNMENT_STRENGTH;
    }

    match intent {
        FlockIntent::Settling => {
            // Aim at this bird's own patch of the landing ground and sink.
            let target = anchor + bird.landing_offset;
            steering += (target - bird.position).normalize_or_zero() * 3.0;
            steering -= up * 2.2;
        }
        FlockIntent::Lifting => {
            steering += up * 7.0;
        }
        _ => {
            steering += (anchor - bird.position).normalize_or_zero() * FLOCK_ANCHOR_STRENGTH;
            if let Some(surface_radius) = surface_radius {
                let altitude = bird.position.length() - surface_radius;
                let error = cruise_altitude_meters - altitude;
                // Undamped, and knowingly so. This is a pure spring -- force
                // proportional to the error with nothing opposing the velocity
                // -- so a cruising bird porpoises through 1.64m with a period
                // of about six seconds and never settles. Measured, and worse
                // than a plain spring: at 3m/s of climb through the cruise
                // height the rest of the steering *assists* by 2.2, so the ring
                // is driven rather than merely undamped.
                //
                // Adding `- climb_rate * 1.75` does fix it, to 0.85m, and was
                // tried. It also changes the emergent shape of a flock enough
                // to break two invariants that have nothing obviously to do
                // with altitude: flockmates in shot fall from 99.5% to 91.9%,
                // and settled birds close to 0.086m of each other. Vertical
                // spread is doing load-bearing work for the horizontal rules,
                // which is not a thing to correct in passing. Left alone until
                // it can be done with those rules rather than against them.
                //
                // Note for whoever picks this up: 0.16Hz is not what a player
                // reports as birds bobbing with their wingbeat. That was the
                // 30Hz render staircase, fixed separately by interpolation.
                steering += up * (error * ALTITUDE_HOLD_STRENGTH).clamp(-6.0, 6.0);
            }
            // A slow circular drift keeps a cruising flock from freezing into a
            // straight line once the three rules balance.
            let (east, north) = tangent_basis(up);
            let phase = wander_phase + index as f64 * 0.7;
            steering += (east * phase.cos() + north * phase.sin()) * WANDER_STRENGTH;
        }
    }

    steering
}

fn step_flying_bird(
    bird: &mut Bird,
    step_seconds: f64,
    steering: DVec3,
    up: DVec3,
    surface_radius: Option<f64>,
    intent: FlockIntent,
) {
    bird.velocity += limit(steering, 26.0) * step_seconds;

    let speed = bird.velocity.length();
    let (minimum, maximum) = match bird.activity {
        // A landing bird is allowed to get slow; that is what landing is.
        BirdActivity::Landing => (0.0, CRUISE_SPEED_METERS_PER_SECOND),
        BirdActivity::TakingOff => (
            MIN_FLYING_SPEED_METERS_PER_SECOND,
            MAX_SPEED_METERS_PER_SECOND,
        ),
        _ => (
            MIN_FLYING_SPEED_METERS_PER_SECOND,
            MAX_SPEED_METERS_PER_SECOND,
        ),
    };
    if speed > maximum {
        bird.velocity *= maximum / speed;
    } else if speed < minimum && speed > 1.0e-6 {
        bird.velocity *= minimum / speed;
    }

    bird.position += bird.velocity * step_seconds;

    if let Some(surface_radius) = surface_radius {
        let altitude = bird.position.length() - surface_radius;
        match bird.activity {
            BirdActivity::Landing => {
                if altitude <= TOUCHDOWN_ALTITUDE_METERS
                    && bird.velocity.length() <= TOUCHDOWN_SPEED_METERS_PER_SECOND
                {
                    bird.activity = BirdActivity::Walking;
                    bird.activity_seconds = 0.0;
                    bird.position =
                        bird.position.normalize() * (surface_radius + FOOT_CLEARANCE_METERS);
                    // Keep the track, drop the sink rate.
                    let flat = bird.velocity - up * bird.velocity.dot(up);
                    bird.velocity = flat.normalize_or_zero() * WALK_SPEED_METERS_PER_SECOND;
                } else if altitude < 0.0 {
                    // Undershot: hold it just above the ground and let the next
                    // step meet the touchdown test properly.
                    bird.position =
                        bird.position.normalize() * (surface_radius + TOUCHDOWN_ALTITUDE_METERS);
                    bird.velocity -= up * bird.velocity.dot(up);
                }
            }
            _ => {
                // Never let the flocking rules fly a bird into a hillside.
                let floor = surface_radius + 2.0;
                if bird.position.length() < floor {
                    bird.position = bird.position.normalize() * floor;
                    let into_ground = bird.velocity.dot(up).min(0.0);
                    bird.velocity -= up * into_ground;
                }
            }
        }
    }

    let beats = match bird.activity {
        BirdActivity::TakingOff => CLIMB_WINGBEATS_PER_SECOND,
        BirdActivity::Landing => GLIDE_WINGBEATS_PER_SECOND,
        _ => {
            if intent == FlockIntent::Lifting {
                CLIMB_WINGBEATS_PER_SECOND
            } else {
                CRUISE_WINGBEATS_PER_SECOND
            }
        }
    };
    bird.wing_phase = (bird.wing_phase + beats * step_seconds as f32).fract();
}

/// Tangential push away from crowded neighbours, in the ground plane so it
/// cannot lift a bird off its feet or drive it into the ground.
fn walking_separation(bird: &Bird, index: usize, snapshot: &[(DVec3, DVec3)], up: DVec3) -> DVec3 {
    let mut push = DVec3::ZERO;
    for (other_index, (position, _)) in snapshot.iter().enumerate() {
        if other_index == index {
            continue;
        }
        let offset = bird.position - *position;
        let distance = offset.length();
        if distance >= WALK_SEPARATION_METERS || distance <= 1.0e-6 {
            continue;
        }
        let flat = offset - up * offset.dot(up);
        if flat.length_squared() <= 1.0e-12 {
            continue;
        }
        // Linear in the overlap, so a bird that is merely close is nudged and
        // one that is nearly co-located is moved firmly.
        push += flat.normalize() * ((WALK_SEPARATION_METERS - distance) / WALK_SEPARATION_METERS);
    }
    push * WALK_SEPARATION_STRENGTH
}

fn step_walking_bird(
    bird: &mut Bird,
    step_seconds: f64,
    surface_radius: Option<f64>,
    up: DVec3,
    intent: FlockIntent,
    separation: DVec3,
) {
    if intent == FlockIntent::Lifting {
        bird.activity = BirdActivity::TakingOff;
        bird.activity_seconds = 0.0;
        bird.velocity = up * 6.0 + bird.velocity.normalize_or_zero() * 3.0;
        return;
    }

    // A walk is a slow tangential drift with pauses, not a constant march.
    // The pause pattern comes from the bird's own phase so no extra state or
    // generator draw is needed per step.
    let stride = ((bird.activity_seconds * 0.9).sin() * 0.5 + 0.5).powf(2.0);
    let flat = bird.velocity - up * bird.velocity.dot(up);
    let heading = flat.normalize_or_zero();
    // A crowded bird steps aside, and may briefly outpace a stroll to do it,
    // rather than sliding through its neighbour.
    bird.velocity = limit(
        heading * (WALK_SPEED_METERS_PER_SECOND * stride) + separation,
        WALK_SPEED_METERS_PER_SECOND * 2.0,
    );
    bird.position += bird.velocity * step_seconds;

    if let Some(surface_radius) = surface_radius {
        bird.position = bird.position.normalize() * (surface_radius + FOOT_CLEARANCE_METERS);
    }
    // Wings are folded: the phase holds rather than winding on.
    bird.wing_phase = 0.0;
}

/// One vertex of the shared low-poly bird.
///
/// `flap` is the wing rig, in three numbers so the shader can drive a two-bone
/// wing without a skeleton or any per-bird geometry:
///   x  which wing: -1 left, 0 body, +1 right
///   y  arm weight: 0 at the shoulder, 1 from the wrist outward
///   z  hand weight: 0 anywhere on the arm, 0 at the wrist, 1 at the tip
///
/// One bone was tried first and reads as an insect: a single rigid plate
/// pivoting at the shoulder is how a dragonfly flies. A bird's hand trails its
/// arm through the stroke, which is what gives the M-shape at the top of the
/// beat and the swept-back look at the bottom.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BirdVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub flap: [f32; 3],
    pub colour: [f32; 3],
}

/// Body length of the modelled bird in local units; the renderer scales it to
/// metres per instance.
/// Where the hand is hinged, in the bird's local frame. `birds.wgsl` carries
/// the same point as `WRIST_LOCAL` and `the_shader_hinges_the_hand_at_the_mesh
/// _wrist` holds the two together: hinge the hand somewhere the geometry has no
/// joint and the wing tears open mid-beat.
pub const WING_WRIST_LOCAL: [f32; 3] = [0.325, 0.048, -0.02];
/// Half the wing's chord at the wrist, fore and aft of the hinge.
const WING_WRIST_CHORD_HALF: f32 = 0.13;
const BODY_PALE: [f32; 3] = [0.80, 0.79, 0.76];
const WING_GREY: [f32; 3] = [0.44, 0.46, 0.50];
const WING_TIP: [f32; 3] = [0.11, 0.11, 0.13];

fn face(
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    flap_a: [f32; 3],
    flap_b: [f32; 3],
    flap_c: [f32; 3],
    colour: [f32; 3],
    out: &mut Vec<BirdVertex>,
) {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let normal = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    let normal = if length > 1.0e-9 {
        [normal[0] / length, normal[1] / length, normal[2] / length]
    } else {
        [0.0, 1.0, 0.0]
    };
    for (position, flap) in [(a, flap_a), (b, flap_b), (c, flap_c)] {
        out.push(BirdVertex {
            position,
            normal,
            flap,
            colour,
        });
    }
}

/// One panel of a wing, emitted twice with opposite winding so a bird passing
/// overhead still has wings under back-face culling. `inboard` and `outboard`
/// are the rig weights at the two ends of the panel.
#[allow(clippy::too_many_arguments)]
fn wing_panel(
    root_front: [f32; 3],
    tip_front: [f32; 3],
    tip_back: [f32; 3],
    root_back: [f32; 3],
    inboard: [f32; 3],
    outboard: [f32; 3],
    colour: [f32; 3],
    out: &mut Vec<BirdVertex>,
) {
    face(
        root_front, tip_front, tip_back, inboard, outboard, outboard, colour, out,
    );
    face(
        root_front, tip_back, root_back, inboard, outboard, inboard, colour, out,
    );
    face(
        tip_back, tip_front, root_front, outboard, outboard, inboard, colour, out,
    );
    face(
        root_back, tip_back, root_front, inboard, outboard, inboard, colour, out,
    );
}

/// Both panels of one wing: the arm from shoulder to wrist, then the hand from
/// wrist to tip. The hand is a separate panel so the shader can hinge it.
fn wing(side: f32, out: &mut Vec<BirdVertex>) {
    let shoulder = [side, 0.0, 0.0];
    let wrist = [side, 1.0, 0.0];
    let tip = [side, 1.0, 1.0];

    let shoulder_front = [0.07 * side, 0.04, 0.17];
    let shoulder_back = [0.07 * side, 0.02, -0.10];
    // Both wrist vertices are the shader's hinge point, one chord half-width
    // either side of it. Deriving them rather than writing them out is what
    // keeps the joint and the hinge in the same place by construction.
    let [wrist_x, wrist_y, wrist_z] = WING_WRIST_LOCAL;
    let wrist_front = [wrist_x * side, wrist_y, wrist_z + WING_WRIST_CHORD_HALF];
    let wrist_back = [wrist_x * side, wrist_y, wrist_z - WING_WRIST_CHORD_HALF];
    let tip_front = [0.62 * side, 0.06, 0.03];
    let tip_back = [0.55 * side, 0.05, -0.19];

    // Winding follows the side so both wings face the same way out.
    if side > 0.0 {
        wing_panel(
            shoulder_front,
            wrist_front,
            wrist_back,
            shoulder_back,
            shoulder,
            wrist,
            WING_GREY,
            out,
        );
        wing_panel(
            wrist_front,
            tip_front,
            tip_back,
            wrist_back,
            wrist,
            tip,
            WING_TIP,
            out,
        );
    } else {
        wing_panel(
            shoulder_back,
            wrist_back,
            wrist_front,
            shoulder_front,
            shoulder,
            wrist,
            WING_GREY,
            out,
        );
        wing_panel(
            wrist_back,
            tip_back,
            tip_front,
            wrist_front,
            wrist,
            tip,
            WING_TIP,
            out,
        );
    }
}

/// The bird itself: a spindle body, two hinged wings and a tail, eighteen
/// triangles in all. Modelled nose-forward along +Z with +Y up, which is the
/// frame `birds.wgsl` rotates into the bird's heading.
pub fn build_mesh() -> Vec<BirdVertex> {
    let mut out = Vec::new();
    let body = [0.0_f32, 0.0, 0.0];

    let nose = [0.0, 0.0, 0.55];
    let tail = [0.0, 0.0, -0.45];
    let top = [0.0, 0.10, 0.05];
    let bottom = [0.0, -0.10, 0.05];
    let left = [-0.09, 0.0, 0.05];
    let right = [0.09, 0.0, 0.05];

    for (a, b, c) in [
        (nose, top, right),
        (nose, right, bottom),
        (nose, bottom, left),
        (nose, left, top),
        (tail, right, top),
        (tail, bottom, right),
        (tail, left, bottom),
        (tail, top, left),
    ] {
        face(a, b, c, body, body, body, BODY_PALE, &mut out);
    }

    wing(1.0, &mut out);
    wing(-1.0, &mut out);

    face(
        tail,
        [-0.17, 0.01, -0.63],
        [0.17, 0.01, -0.63],
        body,
        body,
        body,
        WING_GREY,
        &mut out,
    );
    face(
        tail,
        [0.17, 0.01, -0.63],
        [-0.17, 0.01, -0.63],
        body,
        body,
        body,
        WING_GREY,
        &mut out,
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat, walkable world one planet radius out, so the flock has somewhere
    /// to be without any terrain machinery.
    fn flat_ground(radius: f64) -> impl Fn(DVec3) -> Option<GroundSample> {
        move |_direction| {
            Some(GroundSample {
                surface_radius_meters: radius,
                slope_radians: 0.0,
                walkable: true,
            })
        }
    }

    fn camera_at(radius: f64) -> DVec3 {
        DVec3::new(0.0, 0.0, 1.0) * radius
    }

    #[test]
    fn flocks_spawn_out_of_view_and_retire_once_left_behind() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(7);
        flocks.advance(1.0, camera, &ground);

        assert_eq!(flocks.flock_count(), MAX_NEAR_FLOCKS);
        assert!(flocks.bird_count() >= MAX_NEAR_FLOCKS * FLOCK_MIN_BIRDS);
        for flock in flocks.flocks() {
            let distance = flock.centroid().distance(camera);
            // Spawned beyond the near edge of the shell, allowing for the
            // scatter and the cruise altitude the anchor is lifted by.
            assert!(
                distance > FLOCK_SPAWN_MIN_METERS - 40.0,
                "flock spawned at {distance}m, inside the shell"
            );
            assert!(distance < FLOCK_SPAWN_MAX_METERS + 120.0, "{distance}m");
        }

        // Walk a kilometre away: every original flock is given up, and the
        // population is refilled around the new position rather than trailing.
        let (east, _north) = tangent_basis(camera.normalize());
        let moved = camera + east * 4_000.0;
        flocks.advance(3.0, moved, &ground);
        assert_eq!(flocks.near_flock_count(moved), MAX_NEAR_FLOCKS);
        for flock in flocks.flocks() {
            assert!(flock.centroid().distance(moved) <= FLOCK_DESPAWN_METERS);
        }
    }

    #[test]
    fn a_flock_lands_walks_and_takes_off_again() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(11);

        let mut seen_walking = false;
        let mut seen_takeoff_after_walking = false;
        let mut time = 0.0;
        while time < 240.0 {
            time += 0.1;
            flocks.advance(time, camera, &ground);
            let walking = flocks.birds().filter(|bird| bird.is_grounded()).count();
            if walking > 0 {
                seen_walking = true;
            }
            if seen_walking
                && flocks
                    .birds()
                    .any(|bird| bird.activity == BirdActivity::TakingOff)
            {
                seen_takeoff_after_walking = true;
                break;
            }
        }
        assert!(seen_walking, "no bird ever landed and walked");
        assert!(
            seen_takeoff_after_walking,
            "a settled flock never left the ground again"
        );
    }

    #[test]
    fn walking_birds_stand_on_the_ground_and_flying_birds_stay_above_it() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(23);
        let mut time = 0.0;
        while time < 200.0 {
            time += 0.1;
            flocks.advance(time, camera, &ground);
            for bird in flocks.birds() {
                let altitude = bird.position.length() - radius;
                if bird.is_grounded() {
                    assert!(
                        (altitude - FOOT_CLEARANCE_METERS).abs() < 1.0e-6,
                        "walking bird at {altitude}m"
                    );
                } else {
                    // Landing birds are allowed right down to the touchdown
                    // band; nothing may be underground.
                    assert!(altitude > -1.0e-6, "bird underground at {altitude}m");
                }
            }
        }
    }

    #[test]
    fn a_flock_keeps_together_without_collapsing_onto_one_point() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(5);
        let mut time = 0.0;
        while time < 30.0 {
            time += 0.1;
            flocks.advance(time, camera, &ground);
        }
        for flock in flocks.flocks() {
            let centroid = flock.centroid();
            let spread = flock
                .birds()
                .iter()
                .map(|bird| bird.position.distance(centroid))
                .fold(0.0_f64, f64::max);
            let closest = flock
                .birds()
                .iter()
                .enumerate()
                .flat_map(|(index, bird)| {
                    flock
                        .birds()
                        .iter()
                        .skip(index + 1)
                        .map(move |other| bird.position.distance(other.position))
                })
                .fold(f64::INFINITY, f64::min);
            assert!(spread < 90.0, "flock scattered to {spread}m");
            assert!(closest > 0.25, "birds converged to {closest}m apart");
        }
    }

    #[test]
    fn birds_fly_over_water_but_never_stand_on_it() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        // Open water: resolvable, so a flock can be over it, but never
        // somewhere a bird could put its feet down.
        let open_water = |_direction: DVec3| {
            Some(GroundSample {
                surface_radius_meters: radius,
                slope_radians: 0.0,
                walkable: false,
            })
        };
        let mut flocks = BirdFlocks::new(3);
        let mut time = 0.0;
        while time < 200.0 {
            time += 0.1;
            flocks.advance(time, camera, &open_water);
            assert!(
                flocks.birds().all(|bird| !bird.is_grounded()),
                "a bird stood on open water at {time:.1}s"
            );
        }
        assert!(
            flocks.flock_count() > 0,
            "no flock over water at all, which is what emptied a beach demo"
        );

        // Unresolved terrain is still declined rather than guessed at: without
        // a surface radius the flock has no altitude to hold.
        let unloaded = |_direction: DVec3| None;
        let mut flocks = BirdFlocks::new(3);
        flocks.advance(2.0, camera, &unloaded);
        assert_eq!(flocks.flock_count(), 0);
    }

    #[test]
    fn cruising_flocks_travel_and_pass_close_enough_to_be_seen() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(41);
        let mut closest_approach = f64::INFINITY;
        let mut time = 0.0;
        while time < 90.0 {
            time += 0.1;
            flocks.advance(time, camera, &ground);
            for flock in flocks.flocks() {
                closest_approach = closest_approach.min(flock.centroid().distance(camera));
            }
        }
        // A 0.42m bird needs to be inside roughly 150m to read as a bird rather
        // than a speck at 720p and 60 degrees.
        assert!(
            closest_approach < 150.0,
            "no flock came closer than {closest_approach:.0}m, so they would only ever be specks"
        );
    }

    #[test]
    fn drifting_flocks_never_leave_the_sky_empty_in_front_of_the_camera() {
        // Counting the whole population instead of the near one stalled
        // spawning once flocks drifted: they sat between the draw distance and
        // the despawn distance, alive but invisible, and a `bird_flyby` replay
        // reported six flocks and a hundred birds with none of them drawn.
        //
        // Asserting only "some bird is drawable" is too weak to catch it -- it
        // held by luck on the first seed tried. Assert the policy itself: the
        // near population is topped up for as long as the camera stands there.
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        for seed in [3, 13, 41, 97] {
            let mut flocks = BirdFlocks::new(seed);
            let mut time = 0.0;
            while time < 240.0 {
                time += 0.1;
                flocks.advance(time, camera, &ground);
                assert!(flocks.flock_count() <= MAX_FLOCKS);
                if time > 20.0 {
                    // Topped up to at least the target; inbound drifters can
                    // carry it above, which is fine and not worth suppressing.
                    assert!(
                        flocks.near_flock_count(camera) >= MAX_NEAR_FLOCKS,
                        "seed {seed} let the near population fall to {} at {time:.1}s",
                        flocks.near_flock_count(camera)
                    );
                    assert!(
                        flocks
                            .birds()
                            .any(|bird| bird.position.distance(camera) <= FLOCK_NEAR_RADIUS_METERS),
                        "seed {seed} had no drawable bird at {time:.1}s"
                    );
                }
            }
        }
    }

    #[test]
    fn birds_never_stand_inside_one_another() {
        // The flocking rules apply to birds on the wing. Walking birds used to
        // ignore their neighbours entirely and settled to 0.014m apart, which
        // for a 0.42m bird is one inside another. The spread test above missed
        // it for two hundred steps because it skipped any flock with a grounded
        // bird in it -- exactly the case that was broken.
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut closest = f64::INFINITY;
        let mut settled = f64::INFINITY;
        let mut grounded_seen = false;
        for seed in [3u64, 13, 41, 97] {
            let mut flocks = BirdFlocks::new(seed);
            let mut time = 0.0;
            while time < 240.0 {
                time += 0.1;
                flocks.advance(time, camera, &ground);
                for flock in flocks.flocks() {
                    let birds = flock.birds();
                    grounded_seen |= birds.iter().any(|bird| bird.is_grounded());
                    for (index, bird) in birds.iter().enumerate() {
                        for other in birds.iter().skip(index + 1) {
                            let gap = bird.position.distance(other.position);
                            closest = closest.min(gap);
                            if bird.is_grounded()
                                && other.is_grounded()
                                && bird.activity_seconds > 1.0
                                && other.activity_seconds > 1.0
                            {
                                settled = settled.min(gap);
                            }
                        }
                    }
                }
            }
        }
        assert!(
            grounded_seen,
            "no bird ever landed, so the ground case is untested"
        );
        // Settled birds must be clear of one another: a bird is about 0.5m
        // long, so anything under that is one standing in another. Measured
        // 0.683m with the walking separation in, against 0.014m without it.
        assert!(
            settled > 0.5,
            "two settled birds stood {settled:.3}m apart, inside one another"
        );
        // The overall minimum is looser on purpose. It is the instant a landing
        // bird touches down beside a settled one, before the separation has
        // pushed them apart over the next few steps; measured 0.132m. Pinned so
        // a regression that makes the touchdown itself overlap still shows up.
        assert!(
            closest > 0.1,
            "birds closed to {closest:.3}m even allowing for touchdown"
        );
    }

    #[test]
    fn small_flocks_join_up_and_never_grow_past_the_ceiling() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut merges = 0;
        let mut largest = 0;
        for seed in [3u64, 13, 41, 97, 128] {
            let mut flocks = BirdFlocks::new(seed);
            let mut time = 0.0;
            while time < 300.0 {
                time += 0.1;
                flocks.advance(time, camera, &ground);
                largest = largest.max(flocks.largest_flock());
                // The ceiling is the whole point of the visibility rule, so it
                // is checked every step rather than at the end.
                assert!(
                    flocks.largest_flock() <= FLOCK_MERGE_CEILING_BIRDS,
                    "seed {seed} grew a flock to {} at {time:.1}s",
                    flocks.largest_flock()
                );
            }
            merges += flocks.merge_count();
        }
        // The counter is direct evidence that the path fired; a size threshold
        // would not be, because a merge of two flocks under the visibility
        // limit lands in the same range a single spawn can already produce.
        assert!(merges > 0, "no two flocks ever joined up");
        assert!(
            largest >= FLOCK_MIN_BIRDS * 2,
            "largest flock was only {largest}"
        );
    }

    #[test]
    fn a_flock_at_the_visibility_limit_neither_sees_nor_is_seen() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        for seed in [5u64, 23, 61] {
            let mut flocks = BirdFlocks::new(seed);
            let mut time = 0.0;
            while time < 240.0 {
                time += 0.1;
                flocks.advance(time, camera, &ground);
                for flock in flocks.flocks() {
                    if flock.birds().len() >= FLOCK_MERGE_VISIBILITY_BIRDS {
                        assert!(
                            !flock.is_mergeable(),
                            "a flock of {} was still merging at {time:.1}s",
                            flock.birds().len()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn flocks_that_cannot_merge_keep_out_of_each_other() {
        // Two things were needed and the measurement said which. The steering
        // rule alone changed nothing at all -- 19.77m with it and 19.77m
        // without, bit for bit -- because the closest approach was at t=0.1s,
        // the first step after a spawn, and no amount of steering undoes where
        // a flock was put. With the spawn separation as well the closest
        // approach is 32.61m and happens at t=268.6s, during a real encounter.
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut closest = f64::INFINITY;
        for seed in [3u64, 13, 41, 97, 128] {
            let mut flocks = BirdFlocks::new(seed);
            let mut time = 0.0;
            while time < 300.0 {
                time += 0.1;
                flocks.advance(time, camera, &ground);
                let live = flocks.flocks();
                for (index, flock) in live.iter().enumerate() {
                    for other in live.iter().skip(index + 1) {
                        // Settled flocks are standing on their landing ground
                        // and are not steered, so they are not this rule's.
                        if !matches!(flock.intent, FlockIntent::Cruising | FlockIntent::Lifting)
                            || !matches!(other.intent, FlockIntent::Cruising | FlockIntent::Lifting)
                        {
                            continue;
                        }
                        let could_merge = flock.is_mergeable()
                            && other.is_mergeable()
                            && flock.birds().len() + other.birds().len()
                                <= FLOCK_MERGE_CEILING_BIRDS;
                        if could_merge {
                            continue;
                        }
                        closest = closest.min(flock.centroid().distance(other.centroid()));
                    }
                }
            }
        }
        assert!(
            closest > 25.0,
            "two flocks that cannot merge closed to {closest:.2}m, which is inside a flock"
        );
    }

    #[test]
    fn a_spawn_shell_override_is_honoured_and_a_malformed_one_is_not() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut close = BirdFlocks::new(7).with_spawn_shell(30.0, 60.0);
        close.advance(1.0, camera, &ground);
        assert!(close.flock_count() > 0);
        for flock in close.flocks() {
            let distance = flock.centroid().distance(camera);
            assert!(
                distance < 160.0,
                "override ignored, flock at {distance:.0}m"
            );
        }
        // Reversed, zero and non-finite bounds all leave the shipping shell in
        // place rather than putting a flock on top of the camera.
        for (min, max) in [(60.0, 30.0), (0.0, 50.0), (f64::NAN, 50.0)] {
            let mut guarded = BirdFlocks::new(7).with_spawn_shell(min, max);
            guarded.advance(1.0, camera, &ground);
            for flock in guarded.flocks() {
                let distance = flock.centroid().distance(camera);
                assert!(
                    distance > FLOCK_SPAWN_MIN_METERS - 40.0,
                    "malformed override ({min}, {max}) was applied: flock at {distance:.0}m"
                );
            }
        }
    }

    #[test]
    fn reading_the_spawn_shell_from_the_environment_always_yields_a_usable_set() {
        // A smoke test, and honest as one: the variable may or may not be set in
        // whatever environment the suite runs in, so this pins that the reader
        // never panics and always hands back a set that still spawns. The
        // validation itself is covered above, where the bounds are explicit.
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(19).with_spawn_shell_from_env();
        flocks.advance(1.0, camera, &ground);
        assert!(flocks.flock_count() > 0);
    }

    #[test]
    fn bird_identities_are_unique_and_survive_merges() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut merged = false;
        for seed in [3u64, 13, 41, 97, 128] {
            let mut flocks = BirdFlocks::new(seed);
            let mut time = 0.0;
            while time < 300.0 {
                time += 0.1;
                flocks.advance(time, camera, &ground);
                merged |= flocks.merge_count() > 0;
                let ids: Vec<u64> = flocks.birds().map(|bird| bird.id).collect();
                let unique: std::collections::BTreeSet<u64> = ids.iter().copied().collect();
                assert_eq!(ids.len(), unique.len(), "duplicate bird id at {time:.1}s");
                assert!(!ids.contains(&0), "zero is reserved for no-bird");
            }
        }
        assert!(
            merged,
            "no merge happened, so identity across one was untested"
        );
    }

    #[test]
    fn a_ride_target_is_a_live_bird_and_stays_put_while_it_lives() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(29);
        flocks.advance(1.0, camera, &ground);
        let first = flocks
            .ride_target(camera, None)
            .expect("a flock exists, so a target does");
        assert!(flocks.bird(first).is_some());

        // Held for as long as the bird lives, then handed on to another live
        // bird rather than going blank.
        let mut target = first;
        let mut followed = 0;
        let mut rechosen = 0;
        let mut time = 1.0;
        let (east, _north) = tangent_basis(camera.normalize());
        while time < 200.0 {
            time += 0.1;
            // Walk away partway through, so the followed flock is retired and
            // the re-choosing branch is actually exercised rather than assumed.
            let eye = if time < 100.0 {
                camera
            } else {
                camera + east * 4_000.0
            };
            flocks.advance(time, eye, &ground);
            let alive = flocks.bird(target).is_some();
            let next = flocks
                .ride_target(eye, Some(target))
                .expect("there is always some flock to follow here");
            if alive {
                assert_eq!(next, target, "target moved while its bird still flew");
                followed += 1;
            } else {
                assert_ne!(next, target, "kept a target that no longer exists");
                assert!(flocks.bird(next).is_some(), "chose a bird that is gone");
                rechosen += 1;
            }
            target = next;
        }
        assert!(followed > 100, "only followed for {followed} steps");
        assert!(
            rechosen > 0,
            "no bird was lost, so re-choosing went untested"
        );
    }

    #[test]
    fn a_ride_starts_on_a_rearward_bird_of_the_nearest_flock() {
        // Pressing the key while looking at a flock has to land on *that*
        // flock, and toward the back of it, or the shot opens with the flock
        // behind the camera. Both halves are asserted against every flock the
        // simulation actually produces rather than a constructed one.
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(11);
        let mut checked = 0u32;
        let mut multi_flock = 0u32;
        let mut time = 0.0;

        while time < 90.0 {
            time += 0.5;
            flocks.advance(time, camera, &ground);
            // A fresh pick every step: this is the key-press path, not the
            // held-target path the neighbouring test covers.
            let Some(id) = flocks.ride_target(camera, None) else {
                continue;
            };
            let flock = flocks
                .flocks()
                .iter()
                .find(|flock| flock.birds().iter().any(|bird| bird.id == id))
                .expect("the ridden bird belongs to a flock");

            // Nearest, measured the same way the caller means it.
            let chosen_range = (flock.centroid() - camera).length();
            let airborne: Vec<&Flock> = flocks
                .flocks()
                .iter()
                .filter(|flock| flock.birds().iter().any(|bird| !bird.is_grounded()))
                .collect();
            if airborne.len() > 1 {
                multi_flock += 1;
            }
            for other in &airborne {
                assert!(
                    (other.centroid() - camera).length() >= chosen_range - 1.0e-9,
                    "rode a flock that was not the nearest"
                );
            }

            // And toward the back of it. Rank by the same axis the selection
            // uses, and require the rider to sit in the rear third.
            let Some(forward) = flock.mean_heading() else {
                continue;
            };
            let centroid = flock.centroid();
            let along = |bird: &Bird| (bird.position - centroid).dot(forward);
            let rider = flock
                .birds()
                .iter()
                .find(|bird| bird.id == id)
                .expect("the rider");
            let flying = flock
                .birds()
                .iter()
                .filter(|bird| !bird.is_grounded())
                .count();
            let ahead_of_rider = flock
                .birds()
                .iter()
                .filter(|bird| !bird.is_grounded())
                .filter(|bird| along(bird) > along(rider))
                .count();
            assert!(
                ahead_of_rider + 1 > flying * 2 / 3,
                "rode a bird with only {ahead_of_rider} of {flying} flockmates ahead of it"
            );
            assert!(!rider.is_grounded(), "rode a bird standing on the ground");
            checked += 1;
        }

        assert!(checked > 100, "only {checked} picks were checked");
        assert!(
            multi_flock > 10,
            "only {multi_flock} picks had a rival flock, so nearest went untested"
        );
    }

    #[test]
    fn birds_are_drawn_between_steps_rather_than_on_them() {
        // The flock is simulated at a fixed 30Hz so replays land on the same
        // birds. Frames are not drawn at 30Hz, so the drawn pose has to be
        // interpolated or it repeats for several frames and then jumps.
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(29);
        flocks.advance(4.0, camera, &ground);
        let target = flocks
            .birds()
            .find(|bird| !bird.is_grounded())
            .expect("a flier")
            .id;

        // Sample four times per simulation step.
        let mut drawn = Vec::new();
        let mut time = 4.0;
        while time < 12.0 {
            time += BIRD_FIXED_STEP_SECONDS / 4.0;
            flocks.advance(time, camera, &ground);
            let alpha = flocks.interpolation_alpha();
            let Some(bird) = flocks.bird(target) else {
                break;
            };
            drawn.push(bird.position_at(alpha));
        }
        assert!(drawn.len() > 500, "only {} samples", drawn.len());

        // Every consecutive pair must differ: a repeated position is a frame
        // that showed the same pose as the last one.
        let repeats = drawn
            .windows(2)
            .filter(|w| (w[1] - w[0]).length() < 1.0e-9)
            .count();
        assert_eq!(
            repeats, 0,
            "{repeats} frames drew a bird that had not moved"
        );

        // And the steps must be even. Raw stepped output moves by a whole
        // step's worth on one frame in four and not at all on the other three,
        // so the largest single move is several times the mean.
        let moves: Vec<f64> = drawn.windows(2).map(|w| (w[1] - w[0]).length()).collect();
        let mean = moves.iter().sum::<f64>() / moves.len() as f64;
        let largest = moves.iter().cloned().fold(0.0_f64, f64::max);
        assert!(
            largest < mean * 2.0,
            "largest frame-to-frame move {largest:.4}m against a mean of {mean:.4}m, \
             which is the 30Hz staircase showing through"
        );
    }

    #[test]
    fn a_wingbeat_never_runs_backwards_across_the_wrap() {
        // The phase is a fraction of a turn, so blending 0.98 -> 0.02 the naive
        // way runs the wings back through a whole beat in one frame.
        let mut bird = Bird {
            id: 0,
            position: DVec3::new(4_000_050.0, 0.0, 0.0),
            velocity: DVec3::ZERO,
            activity: BirdActivity::Flying,
            wing_phase: 0.02,
            previous_position: DVec3::new(4_000_050.0, 0.0, 0.0),
            previous_wing_phase: 0.98,
            landing_offset: DVec3::ZERO,
            activity_seconds: 0.0,
        };
        // Forwards across the wrap: the short way is +0.04, so the midpoint sits
        // just past the wrap, not back at 0.5.
        let mid = bird.wing_phase_at(0.5);
        assert!(
            !(0.01..=0.99).contains(&mid),
            "midpoint {mid} took the long way round the cycle"
        );
        // Backwards across the wrap behaves the same.
        bird.previous_wing_phase = 0.02;
        bird.wing_phase = 0.98;
        let mid = bird.wing_phase_at(0.5);
        assert!(
            !(0.01..=0.99).contains(&mid),
            "midpoint {mid} took the long way round the cycle"
        );
        // And the ends are exact, so a settled frame is not smeared.
        assert!((bird.wing_phase_at(1.0) - 0.98).abs() < 1.0e-6);
        assert!((bird.wing_phase_at(0.0) - 0.02).abs() < 1.0e-6);
    }

    #[test]
    fn a_chase_camera_sits_behind_and_above_its_bird() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(29);
        flocks.advance(2.0, camera, &ground);
        let target = flocks.ride_target(camera, None).expect("a bird to follow");
        let bird = *flocks.bird(target).expect("the target exists");

        let (eye, look_at, up) = flocks
            .chase_camera(target, 2.4, 0.7, 8.0)
            .expect("the target is in a flock");
        let forward = bird.heading(tangent_basis(bird.up()).0);

        // Behind: the eye is on the far side of the bird from its heading.
        assert!((bird.position - eye).dot(forward) > 0.0);
        // And above it, by the offset asked for.
        assert!(((eye - bird.position).dot(bird.up()) - 0.7).abs() < 1.0e-6);
        // Up is the local radial, which is what keeps the horizon level.
        assert!((up - bird.position.normalize()).length() < 1.0e-9);
        // The eye is the requested distance back, allowing for the rise.
        let offset = eye - bird.position;
        assert!((offset.length() - (2.4_f64.powi(2) + 0.7_f64.powi(2)).sqrt()).abs() < 1.0e-6);
        // And the aim is ahead of the bird, never behind it.
        assert!((look_at - bird.position).dot(forward) > 0.0);
    }

    #[test]
    fn the_bird_cam_has_flockmates_in_shot() {
        // Riding the first bird in the vector framed an empty sky whenever that
        // bird was out in front, which is most of the time for a leader. The
        // camera now rides the one at the back and aims at the flock.
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let half_fov = 27.0_f64.to_radians().cos();
        let mut had_company = 0u32;
        let mut aligned = 0u32;
        let mut samples = 0u32;

        for seed in [3u64, 29, 41, 97] {
            let mut flocks = BirdFlocks::new(seed);
            let mut target = None;
            let mut time = 0.0;
            while time < 60.0 {
                time += 0.1;
                flocks.advance(time, camera, &ground);
                target = flocks.ride_target(camera, target);
                let Some(id) = target else { continue };
                let Some((eye, aim, _up)) = flocks.chase_camera(id, 2.4, 0.7, 8.0) else {
                    continue;
                };
                let view = (aim - eye).normalize();
                let flock = flocks
                    .flocks()
                    .iter()
                    .find(|flock| flock.birds().iter().any(|bird| bird.id == id))
                    .expect("the target's flock");
                if flock.birds().len() < 4 {
                    continue;
                }
                // Flockmates inside a 54 degree frame, ahead of the camera.
                let in_shot = flock
                    .birds()
                    .iter()
                    .filter(|bird| bird.id != id)
                    .filter(|bird| {
                        let offset = bird.position - eye;
                        offset.length() > 1.0e-6 && offset.normalize().dot(view) > half_fov
                    })
                    .count();
                if in_shot > 0 {
                    had_company += 1;
                }
                let bird = flocks.bird(id).expect("the target");
                let heading = bird.heading(tangent_basis(bird.up()).0);
                if view.dot(heading) > 0.0 {
                    aligned += 1;
                }
                samples += 1;
            }
        }

        assert!(samples > 500, "only {samples} frames sampled");
        // The aim is clamped ahead of the bird, so looking the way it flies is
        // an invariant rather than a tendency.
        assert_eq!(
            aligned, samples,
            "the camera looked back down the flock on some frame"
        );
        // Company is not an invariant and cannot be: when the flock really is
        // behind the bird, a camera that still looks forward sees empty sky.
        // Aiming straight at the centroid instead buys company on nearly every
        // frame but points the camera backwards on 42% of them, which reads as
        // being dragged along rather than flying with them.
        //
        // 99.5% measured with the rear-third, most-central pick. The bar below
        // is set where only that pick clears it: on this same run, riding
        // whichever bird happens to be first in the flock's vector gives 83.6%,
        // the most central bird of the whole flock 86.0%, and the single
        // rearmost bird with no centring 89.9%. Both halves of the rule earn
        // their place.
        let with_company = 100.0 * f64::from(had_company) / f64::from(samples);
        assert!(
            with_company > 95.0,
            "only {with_company:.1}% of frames had a flockmate in shot"
        );
        // Not every frame, and it cannot be: the target is deliberately held
        // rather than re-picked, since re-picking would cut between birds
        // continuously, so a bird chosen at the back can drift forward later.
        // This is a measured quality bar, not an invariant claimed.
    }

    #[test]
    fn every_flock_reports_a_centroid_among_its_own_birds() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let mut flocks = BirdFlocks::new(31);
        assert_eq!(flocks.flock_centroids().count(), 0, "no flocks yet");

        let mut time = 0.0;
        while time < 60.0 {
            time += 0.1;
            flocks.advance(time, camera, &ground);
            let centroids: Vec<DVec3> = flocks.flock_centroids().collect();
            assert_eq!(centroids.len(), flocks.flock_count());
            // Each centre sits inside its own flock's spread, which is what
            // makes it a sensible thing to point a marker at.
            for (centroid, flock) in centroids.iter().zip(flocks.flocks()) {
                let furthest = flock
                    .birds()
                    .iter()
                    .map(|bird| bird.position.distance(*centroid))
                    .fold(0.0_f64, f64::max);
                assert!(furthest < 120.0, "centroid {furthest:.0}m from its flock");
            }
        }
    }

    #[test]
    fn the_same_seed_replays_the_same_flock() {
        let radius = 4_000_000.0;
        let camera = camera_at(radius + 2.0);
        let ground = flat_ground(radius);
        let run = || {
            let mut flocks = BirdFlocks::new(99);
            let mut time = 0.0;
            while time < 12.0 {
                time += 0.1;
                flocks.advance(time, camera, &ground);
            }
            flocks
                .birds()
                .map(|bird| bird.position)
                .collect::<Vec<DVec3>>()
        };
        assert_eq!(run(), run());
    }
}
