//! Low-poly ship hull, and the rigid body that floats it on the analytic ocean.
//!
//! Like `surface_camera`, this module owns only medium-independent geometry and
//! physics so it can be tested without a GPU. The caller supplies the water
//! surface as a closure over planet-local directions, which keeps the ocean's
//! Gerstner field -- and any synthetic field a test wants to drive the hull
//! with -- out of the model itself.
//!
//! Ship-local axes are +X forward to the bow, +Y to port, +Z up, with the
//! origin on the design waterline amidships. Positions and orientations are
//! planet-local f64: at a 4,000km radius an f32 has roughly half a metre of
//! resolution, so nothing here may round to f32 before it has been made
//! camera-relative.

use glam::{DMat3, DQuat, DVec3};

use crate::planet::planet_radius_meters;
use crate::surface_camera::GRAVITY_METERS_PER_SECOND_SQUARED;

/// How big the ship is against the 42m coaster it was designed and tuned as.
///
/// Set this rather than any single dimension. Every length in the hull and its
/// superstructure is multiplied by it, and the float is carried along by Froude
/// similarity -- lengths by the scale, times by its square root -- so a larger
/// ship heaves and rolls like the same ship seen larger rather than like a
/// stiffer or livelier one. That is why the metacentric height and the drag fade
/// grow with it, the heave drag per square metre grows with its root, and the
/// roll and yaw damping rates shrink by its root.
///
/// Surge damping is left alone. It stands in for the ship riding at anchor, not
/// for anything the hull's size sets, and scaled with the rest it let a
/// twice-size hull drift 51m in 50s on the real sea against 36m at scale 1.
/// Figures quoted in the comments below were measured at scale 1.
pub const SHIP_SCALE: f64 = 0.25;
/// `sqrt(SHIP_SCALE)`, the factor Froude similarity stretches time by. Spelt out
/// because `sqrt` is not const; the assertion keeps the two in step.
const SHIP_TIME_SCALE: f64 = 0.5;
const _: () = assert!(
    (SHIP_TIME_SCALE * SHIP_TIME_SCALE - SHIP_SCALE).abs() < 1.0e-12,
    "SHIP_TIME_SCALE must be the square root of SHIP_SCALE"
);

pub const HULL_LENGTH_METERS: f64 = 42.0 * SHIP_SCALE;
/// The hull's splashes -- spray, the foam band round it, and the slam and
/// impact thresholds that start them -- were tuned against the scale-2 hull,
/// 84m long. They follow the hull from there as the float does: lengths by
/// `SPLASH_LENGTH_SCALE`, speeds and times by its square root
/// (`splash_speed_scale`). The spray and foam shaders take the same ratio from
/// the hull half-length they are sent, over this.
pub const SPLASH_TUNED_HALF_LENGTH_METERS: f64 = 42.0;
pub const SPLASH_LENGTH_SCALE: f64 = 0.5 * HULL_LENGTH_METERS / SPLASH_TUNED_HALF_LENGTH_METERS;

pub fn splash_speed_scale() -> f64 {
    SPLASH_LENGTH_SCALE.sqrt()
}
pub const HULL_BEAM_METERS: f64 = 11.0 * SHIP_SCALE;
/// Keel below the design waterline amidships.
pub const HULL_DRAFT_METERS: f64 = 3.0 * SHIP_SCALE;
/// Deck above the design waterline amidships.
pub const HULL_FREEBOARD_METERS: f64 = 3.0 * SHIP_SCALE;

const SEAWATER_DENSITY_KG_PER_CUBIC_METER: f64 = 1025.0;
/// Longitudinal and transverse buoyancy columns. The hull only pitches and
/// rolls with a wave because separate columns see different water heights, so
/// this is the resolution of the float, not just of the volume integral.
const BUOYANCY_STATIONS: usize = 16;
const BUOYANCY_COLUMNS: usize = 5;
/// Vertical drag per square metre of plan area. Chosen for a heave damping
/// ratio near 0.3: enough that a dropped hull settles in a few oscillations
/// rather than ringing, and far short of pinning it to the surface.
const HEAVE_DRAG_KG_PER_SQUARE_METER_SECOND: f64 = 3_100.0 * SHIP_TIME_SCALE;
/// The first metre of immersion fades drag in. A column that switches its drag
/// on at full strength the instant it touches makes a hull chatter along a
/// crest instead of riding it.
const DRAG_IMMERSION_FADE_METERS: f64 = 1.0 * SHIP_SCALE;
/// Horizontal water resistance, as a fraction of the speed *relative to the
/// water* shed per second, in proportion to how much of the hull is in it. The
/// hull carries no propulsion, so this is what stops wave impulses walking it
/// across the ocean. Relative to the water, not to the planet: a floating hull
/// is carried back and forth by the orbital motion under each crest, which is
/// what keeps it riding the same water up and over. Held still instead, it let
/// a 23 m/s crest sweep underneath, so a 38m storm swell drove it under, threw
/// it 26m clear of the water behind the crest and plunged it back in.
/// A hull dragged broadside through water meets a great deal of resistance,
/// from its own drag and from the water it has to shift with it. At 0.35 the
/// wave-slope forcing walked the hull 46m in under a minute. This acts only on
/// horizontal velocity, so it holds the hull roughly where it was moored
/// without touching how it rolls, pitches or yaws there.
const SURGE_DAMPING_PER_SECOND: f64 = 3.0;
/// Yaw is now forced and damped by the same tilted buoyancy as every other
/// axis, so this only bleeds off the slow residual spin that nothing else
/// opposes. Set high enough to stand in for absent forcing, it froze the hull's
/// head in place.
/// A real hull weathervanes: its lateral area resists being turned, which this
/// model has no term for. Left at 0.08 the hull swung its head through 134
/// degrees in a minute, which is a hull with no directional stability at all.
const YAW_DAMPING_PER_SECOND: f64 = 0.4 / SHIP_TIME_SCALE;
/// Eddy and bilge-keel roll damping, as a fraction of roll rate shed per
/// second. The buoyancy columns damp heave and pitch well, because those act
/// over the hull's length; roll acts over its beam and comes out badly
/// under-damped, which is the same reason real hulls carry bilge keels. Without
/// this the hull answers a 23-degree sea with a 55-degree knockdown.
const ROLL_DAMPING_PER_SECOND: f64 = 0.9 / SHIP_TIME_SCALE;
/// Metacentric height: the single number that sets how a hull rolls. The mass
/// centre is then placed to produce it, rather than the other way round.
///
/// With mass at the buoyancy centre, as the first cut had it, GM is the whole
/// metacentric radius -- 2.8m on this form, well over a real coaster's -- and
/// the hull is far too stiff to roll, snapping upright in a couple of seconds
/// and reading as welded to the water. Loading it until GM nearly vanished
/// instead let a 23-degree sea knock it down to 55. Small cargo ships run
/// around 0.5 to 1.5m; roll period grows as 1/sqrt(GM).
const METACENTRIC_HEIGHT_METERS: f64 = 0.9 * SHIP_SCALE;
/// Below this the prism model of a buoyancy column stops describing anything,
/// so its displacement is bounded rather than allowed to run away.
const MINIMUM_COLUMN_TILT_COSINE: f64 = 0.2;
/// The float integrates only whole steps of this length, so the trajectory
/// does not depend on how a frame happened to be chopped up.
pub const FIXED_STEP_SECONDS: f64 = 1.0 / 120.0;

/// Water at one sample point: surface altitude relative to sea level, and how
/// fast that surface is itself rising.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaterSample {
    pub height_meters: f64,
    pub vertical_velocity_meters_per_second: f64,
    /// Tangential gradient of the surface, in the planet frame: metres of rise
    /// per metre travelled. Tilts the buoyant force off the vertical.
    pub slope: DVec3,
    /// Horizontal velocity of the surface water (planet frame): its orbital
    /// motion under the waves. The hull's horizontal drag is relative to it.
    pub horizontal_velocity: DVec3,
}

/// Normalised station position, -1 at the transom and +1 at the stem.
fn station_parameter(index: usize, count: usize) -> f64 {
    -1.0 + 2.0 * (index as f64) / (count as f64)
}

/// Half-beam at a station. The bow tapers to a point; the transom stays broad.
pub fn half_beam_meters(t: f64) -> f64 {
    let shape = if t >= 0.0 {
        // Beam has to start coming off well before the stem, or the entry
        // reads as a blunt slab: at the old cubic taper the hull still carried
        // 45% of its beam a twentieth of its length from the bow.
        (1.0 - t * t).max(0.0).powf(0.6)
    } else {
        1.0 - 0.2 * t * t
    };
    0.5 * HULL_BEAM_METERS * shape
}

/// How the water surface meets the hull side at station `t`, given the water's
/// height above the design waterline there (`immersion`, metres): `contact` is
/// 1 while the surface lies between keel and deck and fades to 0 as it passes
/// either (hull clear of the water, or buried), and `below_deck` fades to 0 as
/// the surface rises over the deck. Splashes and hull foam need the first;
/// spray thrown at the surface needs the second.
pub fn hull_water_contact(t: f64, immersion: f64) -> (f64, f64) {
    let ramp = |low: f64, high: f64, x: f64| {
        let s = ((x - low) / (high - low)).clamp(0.0, 1.0);
        s * s * (3.0 - 2.0 * s)
    };
    let edge = 0.5 * SPLASH_LENGTH_SCALE;
    let (keel, deck) = (keel_depth_meters(t), sheer_height_meters(t));
    let below_deck = 1.0 - ramp(deck - edge, deck + edge, immersion);
    (ramp(-keel - edge, -keel + edge, immersion) * below_deck, below_deck)
}

/// Keel depth below the design waterline. The forefoot rises toward the stem.
pub fn keel_depth_meters(t: f64) -> f64 {
    HULL_DRAFT_METERS * (1.0 - 0.75 * t.max(0.0).powi(3))
}

/// Deck height above the design waterline, with sheer rising toward both ends.
pub fn sheer_height_meters(t: f64) -> f64 {
    HULL_FREEBOARD_METERS * (1.0 + 0.45 * t * t)
}

/// One vertical prism of hull, from its keel up to its deck.
#[derive(Clone, Copy, Debug)]
struct BuoyancyColumn {
    /// Keel-level centre of the column, in ship-local metres.
    keel_local: DVec3,
    plan_area_square_meters: f64,
    height_meters: f64,
}

/// The hull's mass properties and buoyancy discretisation, built once.
pub struct ShipHull {
    columns: Vec<BuoyancyColumn>,
    mass_kg: f64,
    /// Where the hull's weight acts, in the waterline-origin ship frame.
    centre_of_mass_local: DVec3,
    metacentric_height_meters: f64,
    /// Diagonal of the inertia tensor in ship-local axes.
    inertia_local: DVec3,
}

impl Default for ShipHull {
    fn default() -> Self {
        Self::new()
    }
}

impl ShipHull {
    pub fn new() -> Self {
        let mut columns = Vec::with_capacity(BUOYANCY_STATIONS * BUOYANCY_COLUMNS);
        let station_length = HULL_LENGTH_METERS / BUOYANCY_STATIONS as f64;
        for station in 0..BUOYANCY_STATIONS {
            // Sample the station at its centre so the integral is a midpoint
            // rule rather than a systematic under- or over-estimate.
            let t = station_parameter(station, BUOYANCY_STATIONS) + 1.0 / BUOYANCY_STATIONS as f64;
            let x = 0.5 * HULL_LENGTH_METERS * t;
            let half_beam = half_beam_meters(t);
            if half_beam <= 0.0 {
                continue;
            }
            let column_width = 2.0 * half_beam / BUOYANCY_COLUMNS as f64;
            let keel_depth = keel_depth_meters(t);
            let sheer = sheer_height_meters(t);
            for column in 0..BUOYANCY_COLUMNS {
                let y = -half_beam + column_width * (column as f64 + 0.5);
                columns.push(BuoyancyColumn {
                    keel_local: DVec3::new(x, y, -keel_depth),
                    plan_area_square_meters: station_length * column_width,
                    height_meters: keel_depth + sheer,
                });
            }
        }

        // Mass is the water the hull displaces when it sits exactly on its
        // design waterline, taken from these same columns. Deriving it from the
        // discretisation rather than an independent estimate is what lets
        // `settles_on_its_design_waterline` be an equality rather than a range.
        let displaced_volume: f64 = columns
            .iter()
            .map(|column| column.plan_area_square_meters * -column.keel_local.z)
            .sum();
        let mass_kg = SEAWATER_DENSITY_KG_PER_CUBIC_METER * displaced_volume;

        // The centre of buoyancy at the design waterline. A fine bow and a
        // broad transom put it aft of amidships, so a hull whose mass acts at
        // the origin instead trims itself several degrees down by the head and
        // sinks while doing it. Real hulls are ballasted so weight sits over
        // buoyancy; carrying that offset explicitly is what makes the design
        // waterline an equilibrium rather than a starting guess.
        let centre_of_buoyancy_local = columns
            .iter()
            .map(|column| {
                let immersed = -column.keel_local.z;
                let volume = column.plan_area_square_meters * immersed;
                DVec3::new(column.keel_local.x, column.keel_local.y, -0.5 * immersed) * volume
            })
            .sum::<DVec3>()
            / displaced_volume;
        // Metacentric radius: the waterplane's second moment about the
        // centreline over the displaced volume. Loading the hull until GM hits
        // its target fixes how high the mass rides above the buoyancy centre.
        let transverse_second_moment: f64 = columns
            .iter()
            .map(|column| column.plan_area_square_meters * column.keel_local.y.powi(2))
            .sum();
        let metacentric_radius_meters = transverse_second_moment / displaced_volume;
        let centre_of_mass_local = centre_of_buoyancy_local
            + DVec3::Z * (metacentric_radius_meters - METACENTRIC_HEIGHT_METERS);
        let metacentric_height_meters = METACENTRIC_HEIGHT_METERS;

        // Solid-box inertia over the hull's extents. The float is dominated by
        // waterplane geometry, which the columns already carry exactly; this
        // only sets how briskly the hull answers a righting moment.
        let depth = HULL_DRAFT_METERS + HULL_FREEBOARD_METERS;
        let inertia_local = DVec3::new(
            mass_kg * (HULL_BEAM_METERS * HULL_BEAM_METERS + depth * depth) / 12.0,
            mass_kg * (HULL_LENGTH_METERS * HULL_LENGTH_METERS + depth * depth) / 12.0,
            mass_kg
                * (HULL_LENGTH_METERS * HULL_LENGTH_METERS + HULL_BEAM_METERS * HULL_BEAM_METERS)
                / 12.0,
        );

        Self {
            columns,
            mass_kg,
            centre_of_mass_local,
            metacentric_height_meters,
            inertia_local,
        }
    }

    pub fn mass_kg(&self) -> f64 {
        self.mass_kg
    }

    /// Offset from the ship-local origin, which sits on the design waterline
    /// amidships, to the centre of mass. The renderer needs it to place the
    /// mesh, which is modelled about that origin.
    pub fn centre_of_mass_local(&self) -> DVec3 {
        self.centre_of_mass_local
    }

    /// GM. Positive means the hull rights itself; small means it rolls slowly
    /// and far, large means it snaps back.
    pub fn metacentric_height_meters(&self) -> f64 {
        self.metacentric_height_meters
    }

    pub fn displaced_volume_cubic_meters(&self) -> f64 {
        self.mass_kg / SEAWATER_DENSITY_KG_PER_CUBIC_METER
    }

    pub fn waterplane_area_square_meters(&self) -> f64 {
        self.columns
            .iter()
            .map(|column| column.plan_area_square_meters)
            .sum()
    }
}

/// Rigid-body state, in planet-local metres and radians.
#[derive(Clone, Copy, Debug)]
pub struct ShipBody {
    pub position: DVec3,
    /// Ship-local axes into planet-local axes.
    pub orientation: DQuat,
    pub linear_velocity: DVec3,
    pub angular_velocity: DVec3,
}

impl ShipBody {
    /// Places the hull on its design waterline at `direction`, with its bow on
    /// the given heading. `water_height_meters` is the local surface altitude,
    /// so a hull spawned on a crest starts floating rather than falling to it.
    /// `position` tracks the centre of mass, which sits below that waterline.
    pub fn afloat_at(
        hull: &ShipHull,
        direction: DVec3,
        heading: DVec3,
        water_height_meters: f64,
    ) -> Self {
        let up = direction.normalize();
        let forward = (heading - up * heading.dot(up)).normalize();
        let port = up.cross(forward);
        Self {
            position: up
                * (planet_radius_meters() + water_height_meters + hull.centre_of_mass_local.z),
            orientation: DQuat::from_mat3(&DMat3::from_cols(forward, port, up)),
            linear_velocity: DVec3::ZERO,
            angular_velocity: DVec3::ZERO,
        }
    }

    /// Altitude of the ship-local origin: the point on the hull that should sit
    /// level with the water when it floats at its design draft.
    pub fn waterline_altitude_meters(&self, hull: &ShipHull) -> f64 {
        (self.position + self.orientation * -hull.centre_of_mass_local).length()
            - planet_radius_meters()
    }

    pub fn up(&self) -> DVec3 {
        self.orientation * DVec3::Z
    }

    pub fn forward(&self) -> DVec3 {
        self.orientation * DVec3::X
    }

    /// Angle between the hull's mast and the local vertical. Covers heel and
    /// trim together, which is what a capsize test actually cares about.
    pub fn tilt_radians(&self) -> f64 {
        self.up()
            .dot(self.position.normalize())
            .clamp(-1.0, 1.0)
            .acos()
    }

    /// Advances the float. `water` receives a planet-local unit direction and
    /// returns the surface there.
    pub fn advance(
        &mut self,
        hull: &ShipHull,
        delta_seconds: f64,
        water: impl Fn(DVec3) -> WaterSample,
    ) {
        // Whole steps, counted rather than subtracted down: `remaining -= step`
        // leaves a femtosecond behind that runs as a ninth micro-step, so the
        // same span integrated in one call and in eight did not quite agree.
        // Any sub-step remainder is the caller's to carry -- it keeps the
        // clock, and handing over whole steps is what makes the float
        // independent of where frame boundaries fall.
        let steps = (delta_seconds.max(0.0) / FIXED_STEP_SECONDS + 1.0e-9).floor() as u64;
        for _ in 0..steps {
            self.advance_step(hull, FIXED_STEP_SECONDS, &water);
        }
    }

    fn advance_step(
        &mut self,
        hull: &ShipHull,
        step_seconds: f64,
        water: &impl Fn(DVec3) -> WaterSample,
    ) {
        let radial = self.position.normalize();
        let rotation = DMat3::from_quat(self.orientation);
        let ship_up = rotation * DVec3::Z;
        let mut force = radial * (-GRAVITY_METERS_PER_SECOND_SQUARED * hull.mass_kg);
        let mut torque = DVec3::ZERO;
        let mut submerged_volume = 0.0;
        let mut water_horizontal = DVec3::ZERO;

        for column in &hull.columns {
            let keel_offset = rotation * (column.keel_local - hull.centre_of_mass_local);
            let keel_world = self.position + keel_offset;
            let column_direction = keel_world.normalize();
            let keel_altitude = keel_world.length() - planet_radius_meters();
            let sample = water(column_direction);
            let vertical_depth = sample.height_meters - keel_altitude;
            if vertical_depth <= 0.0 {
                continue;
            }
            // The column is a prism fixed in the hull, so once the hull heels
            // its axis no longer points at the surface. Its submerged length is
            // the vertical depth divided by that tilt, not the depth itself:
            // without this a hull heeled 35 degrees keeps only cos(35) of its
            // displacement, sinks, heels further, and capsizes. The floor
            // bounds the prism model where it stops being meaningful, near
            // beam-on.
            let axis_tilt_cosine = ship_up
                .dot(column_direction)
                .max(MINIMUM_COLUMN_TILT_COSINE);
            let immersion = (vertical_depth / axis_tilt_cosine).min(column.height_meters);
            let volume = column.plan_area_square_meters * immersion;
            submerged_volume += volume;
            water_horizontal += sample.horizontal_velocity * volume;

            // Archimedes on the submerged part of this column, acting at the
            // centroid of that part. Applying it at the keel instead would
            // fake extra stability by pretending the hull's buoyancy sits
            // lower in the water than it does.
            let buoyant_newtons = SEAWATER_DENSITY_KG_PER_CUBIC_METER
                * GRAVITY_METERS_PER_SECOND_SQUARED
                * column.plan_area_square_meters
                * immersion;
            let centroid_offset = keel_offset + ship_up * (0.5 * immersion);
            // Buoyancy is normal to the water surface, not to the planet. On a
            // wave face that gives a horizontal component, which is what surges
            // the hull along the slope and -- because the slope differs from
            // bow to stern -- what swings its head round.
            let surface_normal = (column_direction - sample.slope).normalize();
            let mut column_force = surface_normal * buoyant_newtons;

            let column_velocity =
                self.linear_velocity + self.angular_velocity.cross(centroid_offset);
            let relative_vertical =
                column_velocity.dot(column_direction) - sample.vertical_velocity_meters_per_second;
            let drag_fade = (immersion / DRAG_IMMERSION_FADE_METERS).min(1.0);
            column_force -= column_direction
                * (HEAVE_DRAG_KG_PER_SQUARE_METER_SECOND
                    * column.plan_area_square_meters
                    * drag_fade
                    * relative_vertical);

            force += column_force;
            torque += centroid_offset.cross(column_force);
        }

        self.linear_velocity += force / hull.mass_kg * step_seconds;
        // Surge damping is horizontal only: vertical resistance already comes
        // from the columns, and damping it twice would sink the hull into a
        // rising crest. It pulls toward the water's own horizontal motion,
        // averaged over the submerged volume, as strongly as the hull is wet.
        let vertical_velocity = radial * self.linear_velocity.dot(radial);
        let horizontal_velocity = self.linear_velocity - vertical_velocity;
        let wetted = (submerged_volume / hull.displaced_volume_cubic_meters()).min(1.0);
        let current = if submerged_volume > 0.0 {
            let mean = water_horizontal / submerged_volume;
            mean - radial * mean.dot(radial)
        } else {
            DVec3::ZERO
        };
        self.linear_velocity = vertical_velocity
            + current
            + (horizontal_velocity - current)
                * (1.0 - SURGE_DAMPING_PER_SECOND * wetted * step_seconds).max(0.0);
        self.position += self.linear_velocity * step_seconds;

        let inverse_inertia =
            rotation * DMat3::from_diagonal(DVec3::ONE / hull.inertia_local) * rotation.transpose();
        self.angular_velocity += inverse_inertia * torque * step_seconds;
        let yaw_velocity = radial * self.angular_velocity.dot(radial);
        self.angular_velocity -= yaw_velocity * (YAW_DAMPING_PER_SECOND * step_seconds).min(1.0);
        let ship_forward = rotation * DVec3::X;
        let roll_velocity = ship_forward * self.angular_velocity.dot(ship_forward);
        self.angular_velocity -= roll_velocity * (ROLL_DAMPING_PER_SECOND * step_seconds).min(1.0);

        if self.angular_velocity.length_squared() > 0.0 {
            let spin = DQuat::from_vec4((self.angular_velocity * 0.5 * step_seconds).extend(0.0))
                * self.orientation;
            self.orientation = (self.orientation + spin).normalize();
        }
    }
}

/// One flat-shaded triangle vertex of the hull mesh.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShipVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub colour: [f32; 3],
    pub _padding: f32,
}

pub(crate) fn push_triangle(vertices: &mut Vec<ShipVertex>, a: DVec3, b: DVec3, c: DVec3, colour: [f32; 3]) {
    let normal = (b - a).cross(c - a);
    if normal.length_squared() <= 0.0 {
        return;
    }
    let normal = normal.normalize().as_vec3().to_array();
    for point in [a, b, c] {
        vertices.push(ShipVertex {
            position: point.as_vec3().to_array(),
            normal,
            colour,
            _padding: 0.0,
        });
    }
}

pub(crate) fn push_quad(
    vertices: &mut Vec<ShipVertex>,
    a: DVec3,
    b: DVec3,
    c: DVec3,
    d: DVec3,
    colour: [f32; 3],
) {
    push_triangle(vertices, a, b, c, colour);
    push_triangle(vertices, a, c, d, colour);
}

pub(crate) fn push_box(vertices: &mut Vec<ShipVertex>, centre: DVec3, half_extents: DVec3, colour: [f32; 3]) {
    let (x, y, z) = (half_extents.x, half_extents.y, half_extents.z);
    let corner = |sx: f64, sy: f64, sz: f64| centre + DVec3::new(sx * x, sy * y, sz * z);
    // Wound counter-clockwise seen from outside, so the face normals point out.
    push_quad(
        vertices,
        corner(1.0, -1.0, -1.0),
        corner(1.0, 1.0, -1.0),
        corner(1.0, 1.0, 1.0),
        corner(1.0, -1.0, 1.0),
        colour,
    );
    push_quad(
        vertices,
        corner(-1.0, 1.0, -1.0),
        corner(-1.0, -1.0, -1.0),
        corner(-1.0, -1.0, 1.0),
        corner(-1.0, 1.0, 1.0),
        colour,
    );
    push_quad(
        vertices,
        corner(-1.0, 1.0, -1.0),
        corner(-1.0, 1.0, 1.0),
        corner(1.0, 1.0, 1.0),
        corner(1.0, 1.0, -1.0),
        colour,
    );
    push_quad(
        vertices,
        corner(-1.0, -1.0, 1.0),
        corner(-1.0, -1.0, -1.0),
        corner(1.0, -1.0, -1.0),
        corner(1.0, -1.0, 1.0),
        colour,
    );
    push_quad(
        vertices,
        corner(-1.0, -1.0, 1.0),
        corner(1.0, -1.0, 1.0),
        corner(1.0, 1.0, 1.0),
        corner(-1.0, 1.0, 1.0),
        colour,
    );
    push_quad(
        vertices,
        corner(-1.0, 1.0, -1.0),
        corner(1.0, 1.0, -1.0),
        corner(1.0, -1.0, -1.0),
        corner(-1.0, -1.0, -1.0),
        colour,
    );
}

/// The ship as a flat-shaded triangle list in ship-local metres: the hull body
/// followed by the fittings (`ship_model`). Faces are wound counter-clockwise
/// from outside; sails and flags carry both windings.
pub fn build_mesh() -> Vec<ShipVertex> {
    let model = crate::ship_model::build();
    let mut mesh = model.hull;
    mesh.extend(model.fittings);
    mesh
}

#[cfg(test)]
mod tests {
    use glam::{DQuat, DVec3};

    use crate::ocean;

    use super::{
        HULL_BEAM_METERS, HULL_DRAFT_METERS, HULL_FREEBOARD_METERS, HULL_LENGTH_METERS, SHIP_SCALE,
        SHIP_TIME_SCALE, ShipBody, ShipHull, WaterSample, build_mesh, half_beam_meters,
        keel_depth_meters, sheer_height_meters,
    };
    use crate::planet::planet_radius_meters;

    const START_DIRECTION: DVec3 = DVec3::new(0.838, 0.502, 0.2125);

    fn still_water(height_meters: f64) -> impl Fn(DVec3) -> WaterSample {
        move |_| WaterSample {
            height_meters,
            vertical_velocity_meters_per_second: 0.0,
            slope: DVec3::ZERO,
            horizontal_velocity: DVec3::ZERO,
        }
    }

    fn afloat() -> (ShipHull, ShipBody) {
        let hull = ShipHull::new();
        let body = ShipBody::afloat_at(&hull, START_DIRECTION, DVec3::new(0.0, 1.0, 0.0), 0.0);
        (hull, body)
    }

    #[test]
    fn hull_displaces_its_own_mass_at_the_design_waterline() {
        let hull = ShipHull::new();
        // A 42x11m hull at 3m draft: a few hundred cubic metres, not tens or
        // tens of thousands. This is the sanity bound on the whole float, and
        // like any volume it goes as the cube of `SHIP_SCALE`.
        let volume = hull.displaced_volume_cubic_meters();
        let scale_cubed = SHIP_SCALE.powi(3);
        assert!(
            (600.0 * scale_cubed..1200.0 * scale_cubed).contains(&volume),
            "displaced volume {volume} m3 is not ship-like"
        );
        // Waterplane area cannot exceed the enclosing rectangle, and a hull
        // that tapers to a stem must be comfortably under it.
        let waterplane = hull.waterplane_area_square_meters();
        assert!(waterplane < HULL_LENGTH_METERS * HULL_BEAM_METERS);
        assert!(waterplane > 0.6 * HULL_LENGTH_METERS * HULL_BEAM_METERS);
    }

    #[test]
    fn settles_on_its_design_waterline_in_still_water() {
        let (hull, mut body) = afloat();
        body.advance(&hull, 60.0, still_water(0.0));
        // Mass came from the same columns, so equilibrium is the waterline
        // itself, not merely somewhere near it.
        assert!(
            body.waterline_altitude_meters(&hull).abs() < 0.02,
            "settled at {}m instead of the design waterline",
            body.waterline_altitude_meters(&hull)
        );
        assert!(body.tilt_radians().to_degrees() < 0.1);
        assert!(body.linear_velocity.length() < 0.05);
    }

    #[test]
    fn a_hull_dropped_above_the_water_settles_rather_than_ringing() {
        let (hull, mut body) = afloat();
        body.position = START_DIRECTION.normalize()
            * (planet_radius_meters() + 6.0 * SHIP_SCALE + hull.centre_of_mass_local().z);
        let mut extremes = 0;
        let mut previous_altitude = body.waterline_altitude_meters(&hull);
        let mut rising = false;
        // Ten seconds, a 6m drop and a 5cm settle at scale 1, restated in Froude
        // units: a bigger hull bobs more slowly, by the root of its scale, and
        // settling to within 5cm of a 3m draft is the same as 10cm of a 6m one.
        let steps = (600.0 * SHIP_SCALE.sqrt()).round() as usize;
        for _ in 0..steps {
            body.advance(&hull, 1.0 / 60.0, still_water(0.0));
            let altitude = body.waterline_altitude_meters(&hull);
            let now_rising = altitude > previous_altitude;
            if now_rising != rising {
                extremes += 1;
                rising = now_rising;
            }
            previous_altitude = altitude;
        }
        // Underdamped enough to bob, damped enough to stop: a handful of
        // reversals over ten seconds, not dozens and not zero.
        assert!(
            (2..=12).contains(&extremes),
            "{extremes} direction changes is not a settling bob"
        );
        assert!(body.waterline_altitude_meters(&hull).abs() < 0.05 * SHIP_SCALE);
    }

    #[test]
    fn a_heeled_hull_rights_itself() {
        let (hull, mut body) = afloat();
        let forward = body.forward();
        body.orientation = DQuat::from_axis_angle(forward, 35_f64.to_radians()) * body.orientation;
        let heeled = body.tilt_radians();
        assert!(heeled.to_degrees() > 30.0);
        body.advance(&hull, 90.0, still_water(0.0));
        assert!(
            body.tilt_radians().to_degrees() < 1.0,
            "still heeled {} degrees",
            body.tilt_radians().to_degrees()
        );
    }

    #[test]
    fn the_hull_follows_a_rising_surface() {
        let (hull, mut body) = afloat();
        // A surface climbing slowly enough that a floating hull tracks it.
        let climb_rate = 0.5;
        let mut elapsed = 0.0;
        for _ in 0..1200 {
            let height = climb_rate * elapsed;
            body.advance(&hull, 1.0 / 60.0, move |_| WaterSample {
                height_meters: height,
                vertical_velocity_meters_per_second: climb_rate,
                slope: DVec3::ZERO,
                horizontal_velocity: DVec3::ZERO,
            });
            elapsed += 1.0 / 60.0;
        }
        let surface = climb_rate * elapsed;
        let lag = surface - body.waterline_altitude_meters(&hull);
        // It rides the surface with a small steady lag, rather than being left
        // behind by it or pinned rigidly to it.
        assert!(lag.abs() < 0.5, "hull lags the surface by {lag}m");
    }

    #[test]
    fn the_hull_pitches_toward_a_sloped_surface() {
        let (hull, mut body) = afloat();
        let forward = body.forward();
        let radial = body.position.normalize();
        // A surface tilted along the hull's length: bow-up water forward.
        let slope = 0.06;
        body.advance(&hull, 40.0, |direction| {
            let along = direction.dot(forward) * planet_radius_meters();
            WaterSample {
                height_meters: slope * along,
                vertical_velocity_meters_per_second: 0.0,
                slope: DVec3::ZERO,
                horizontal_velocity: DVec3::ZERO,
            }
        });
        let pitch = body.forward().dot(radial).asin();
        // The bow should have lifted, and by an angle comparable to the slope.
        assert!(pitch > 0.0, "hull pitched {pitch} rad, expected bow-up");
        assert!(
            (pitch - slope.atan()).abs() < 0.02,
            "pitch {pitch} rad does not match the {slope} surface slope"
        );
    }

    #[test]
    fn a_sea_at_the_breaking_limit_cannot_capsize_or_launch_the_hull() {
        let (hull, mut body) = afloat();
        // The worst sea that can physically exist: water waves break past a
        // height-to-length ratio near 1/7, so a 90m wave tops out around 12.9m
        // crest to trough. Driving something steeper would only prove the hull
        // loses to a wave the ocean cannot make.
        const WAVELENGTH_METERS: f64 = 90.0;
        const AMPLITUDE_METERS: f64 = 6.0;
        const {
            assert!(2.0 * AMPLITUDE_METERS / WAVELENGTH_METERS < 1.0 / 7.0);
        }
        let wave_number = std::f64::consts::TAU / WAVELENGTH_METERS;
        // A surface of this steepness reaches `amplitude * wave_number` of
        // slope, and a hull follows the surface it floats on, so this is the
        // tilt to expect before any dynamic overshoot.
        let surface_slope_degrees = (AMPLITUDE_METERS * wave_number).atan().to_degrees();
        let mut peak_tilt_degrees: f64 = 0.0;
        let mut elapsed = 0.0;
        for _ in 0..3600 {
            body.advance(&hull, 1.0 / 60.0, move |direction| {
                // A steep short swell crossing the hull diagonally.
                let phase =
                    direction.dot(DVec3::new(0.6, 0.5, 0.62).normalize()) * planet_radius_meters();
                WaterSample {
                    height_meters: AMPLITUDE_METERS * (wave_number * phase + 1.6 * elapsed).sin(),
                    vertical_velocity_meters_per_second: AMPLITUDE_METERS
                        * 1.6
                        * (wave_number * phase + 1.6 * elapsed).cos(),
                    slope: DVec3::ZERO,
                    horizontal_velocity: DVec3::ZERO,
                }
            });
            elapsed += 1.0 / 60.0;
            peak_tilt_degrees = peak_tilt_degrees.max(body.tilt_radians().to_degrees());
            assert!(body.position.is_finite() && body.orientation.is_finite());
            // A hull driven near its roll period answers with more heel than
            // the wave has slope -- that is resonance, not a fault, and a soft
            // enough GM is exactly what allows it. What must not happen is a
            // knockdown: past roughly this angle the deck edge is buried, the
            // righting arm is falling away, and a real hull of this form is in
            // real trouble.
            assert!(
                body.tilt_radians().to_degrees() < 55.0,
                "hull was knocked down to {} degrees on a {surface_slope_degrees} degree surface",
                body.tilt_radians().to_degrees()
            );
            // It rides the wave rather than being thrown clear of it: the
            // crest is the ceiling, with room for the hull to overshoot.
            assert!(
                body.waterline_altitude_meters(&hull).abs() < 2.0 * AMPLITUDE_METERS,
                "hull reached {}m altitude on a {AMPLITUDE_METERS}m wave",
                body.waterline_altitude_meters(&hull)
            );
        }
        // And it does heel: a hull that sat flat through this would mean the
        // columns were not resolving the wave at all.
        assert!(
            peak_tilt_degrees > 0.3 * surface_slope_degrees,
            "hull only reached {peak_tilt_degrees} degrees on a {surface_slope_degrees} degree sea"
        );
        // The real proof of stability is that a minute of that sea leaves the
        // hull able to stand back up, rather than merely bounded while driven.
        body.advance(&hull, 120.0, still_water(0.0));
        assert!(
            body.tilt_radians().to_degrees() < 1.0,
            "hull could not recover from the sea: {} degrees",
            body.tilt_radians().to_degrees()
        );
        assert!(body.waterline_altitude_meters(&hull).abs() < 0.05);
    }

    #[test]
    fn the_hull_rolls_pitches_and_yaws_on_the_real_ocean() {
        // The complaint this test exists for: a hull can float correctly and
        // still read as a box sliding up and down the water. Heave alone is not
        // a float, so all three rotational axes have to answer the real sea,
        // not a synthetic one chosen to make them.
        let hull = ShipHull::new();
        // The band small cargo ships run, at the 42m design size. GM is a length
        // and scales with the hull, which is also what larger coasters carry.
        assert!(
            (0.5 * SHIP_SCALE..=1.5 * SHIP_SCALE).contains(&hull.metacentric_height_meters()),
            "GM {} m is outside the range small cargo ships run at this size",
            hull.metacentric_height_meters()
        );
        let mut body = ShipBody::afloat_at(
            &hull,
            START_DIRECTION,
            START_DIRECTION.normalize().cross(DVec3::Y),
            0.0,
        );
        let start_forward = body.forward();
        let (mut roll, mut pitch, mut yaw) = (Vec::new(), Vec::new(), Vec::new());
        let mut elapsed = 0.0;
        for step in 0..3000 {
            body.advance(&hull, 1.0 / 60.0, |direction| WaterSample {
                height_meters: ocean::global_wave_height_meters(direction, elapsed, 4000.0),
                vertical_velocity_meters_per_second:
                    ocean::global_wave_vertical_velocity_meters_per_second(
                        direction, elapsed, 4000.0,
                    ),
                slope: ocean::global_wave_slope(direction, elapsed, 4000.0),
                horizontal_velocity: DVec3::ZERO,
            });
            elapsed += 1.0 / 60.0;
            // Skip the first second: the hull starts level on a moving sea and
            // its first swing is a transient, not its response.
            if step < 60 {
                continue;
            }
            let up = body.position.normalize();
            let forward = body.forward();
            let port = body.orientation * DVec3::Y;
            pitch.push(forward.dot(up).asin().to_degrees());
            roll.push(port.dot(up).asin().to_degrees());
            let flatten = |v: DVec3| (v - up * v.dot(up)).normalize();
            yaw.push(
                flatten(forward)
                    .dot(flatten(start_forward))
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees(),
            );
        }
        let span = |values: &[f64]| {
            values.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                - values.iter().cloned().fold(f64::INFINITY, f64::min)
        };
        assert!(span(&roll) > 10.0, "roll span only {} deg", span(&roll));
        assert!(span(&pitch) > 8.0, "pitch span only {} deg", span(&pitch));
        // Yaw is the axis buoyancy along the radial cannot drive at all, so
        // this fails outright if the force ever stops following the surface.
        assert!(span(&yaw) > 2.0, "yaw span only {} deg", span(&yaw));

        // It must oscillate, not simply lean over once and stay there.
        let reversals = |values: &[f64]| {
            let mut count = 0;
            let mut rising = values[1] > values[0];
            for pair in values.windows(2) {
                let now = pair[1] > pair[0];
                if now != rising {
                    count += 1;
                    rising = now;
                }
            }
            count
        };
        // Reversals in a fixed window count roll and pitch periods, and those
        // lengthen with the root of the ship's scale: eight at 42m is the same
        // motion as 5.7 on a hull twice the size.
        let minimum_reversals = (8.0 / SHIP_TIME_SCALE) as usize;
        assert!(
            reversals(&roll) > minimum_reversals,
            "roll reversed {} times",
            reversals(&roll)
        );
        assert!(
            reversals(&pitch) > minimum_reversals,
            "pitch reversed {} times",
            reversals(&pitch)
        );

        // And the wave-slope forcing that drives yaw must not walk the hull
        // out of the scene while it does so.
        let drift_meters = (body.position.normalize() - START_DIRECTION.normalize()).length()
            * planet_radius_meters();
        // 50m rather than 25m since the sea's slope doubled. Slope-driven
        // drift goes as the square of steepness, and the measured 33.7m over
        // 50s is 0.67 m/s against a Stokes-drift scale of about 1.0 m/s for
        // this spectrum -- physical, not a runaway. What is being tested is
        // still that the forcing is bounded rather than walking the hull away.
        assert!(drift_meters < 50.0, "hull drifted {drift_meters} m in 50s");
    }

    #[test]
    fn the_float_does_not_depend_on_how_time_is_chopped_up() {
        // The caller hands the hull whole fixed steps and keeps the remainder,
        // so eight steps in one call and eight calls of one must land in the
        // same place. A partial final substep broke that, which is what made
        // the float frame-rate dependent -- and time acceleration only widens
        // the frames it was depending on.
        let (hull, mut single) = afloat();
        let (_, mut chunked) = afloat();
        let water = |direction: DVec3| WaterSample {
            height_meters: 3.0 * (direction.x * 4.0e5).sin(),
            vertical_velocity_meters_per_second: 0.4,
            slope: DVec3::ZERO,
            horizontal_velocity: DVec3::ZERO,
        };
        single.advance(&hull, super::FIXED_STEP_SECONDS * 8.0, water);
        for _ in 0..8 {
            chunked.advance(&hull, super::FIXED_STEP_SECONDS, water);
        }
        assert_eq!(single.position, chunked.position);
        assert_eq!(single.orientation, chunked.orientation);
        assert_eq!(single.linear_velocity, chunked.linear_velocity);
    }

    /// Instrument, not a regression: the ship's track on the FFT sea, stepped
    /// the way `advance_ship` steps it (one orbital-velocity query per step).
    /// `PLANET_OCEAN_FFT=1 cargo test --release -p planet-app ship_track --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ship_track_on_the_real_sea() {
        let hull = ShipHull::new();
        let start = DVec3::new(3_345_724.0, 2_014_892.0, 863_825.0).normalize();
        let east = start.cross(DVec3::Y).normalize();
        let north = start.cross(east).normalize();
        let mut body = ShipBody::afloat_at(&hull, start, east, 0.0);
        let origin = body.position;
        let (mut elapsed, step) = (0.0, super::FIXED_STEP_SECONDS);
        let (mut max_speed, mut max_gap, mut speed_sum, mut samples) = (0.0f64, 0.0f64, 0.0, 0.0);
        let mut settled_speeds: Vec<f64> = Vec::new();
        for index in 0..(300.0 / step) as usize {
            let water_horizontal = ocean::global_wave_horizontal_velocity(
                body.position.normalize(),
                elapsed,
            );
            let time = elapsed;
            body.advance(&hull, step, |direction| WaterSample {
                height_meters: ocean::global_wave_height_meters(direction, time, 250.0),
                vertical_velocity_meters_per_second:
                    ocean::global_wave_vertical_velocity_meters_per_second(direction, time, 4000.0),
                slope: ocean::global_wave_slope(direction, time, 4000.0),
                horizontal_velocity: water_horizontal,
            });
            elapsed += step;
            let up = body.position.normalize();
            let horizontal = |v: DVec3| v - up * v.dot(up);
            let speed = horizontal(body.linear_velocity).length();
            let gap = (horizontal(body.linear_velocity) - horizontal(water_horizontal)).length();
            max_speed = max_speed.max(speed);
            max_gap = max_gap.max(gap);
            speed_sum += speed;
            samples += 1.0;
            // The first 20s hold the spawn transient (hull at rest in moving water).
            if elapsed > 20.0 {
                settled_speeds.push(speed);
            }
            if index % 240 == 0 {
                let offset = body.position - origin;
                println!(
                    "t={elapsed:6.1} east={:8.1} north={:8.1} speed={speed:5.2} water={:5.2} gap={gap:5.2} tilt={:5.1}",
                    offset.dot(east),
                    offset.dot(north),
                    horizontal(water_horizontal).length(),
                    body.tilt_radians().to_degrees(),
                );
            }
        }
        let offset = body.position - origin;
        println!(
            "after 300s: net {:.1}m (east {:.1}, north {:.1}); max speed {max_speed:.2} m/s, mean {:.2}, max |ship - water| {max_gap:.2}",
            offset.length(),
            offset.dot(east),
            offset.dot(north),
            speed_sum / samples,
        );
        settled_speeds.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f64| settled_speeds[((settled_speeds.len() - 1) as f64 * q) as usize];
        println!(
            "settled (t>20s) horizontal speed over the planet-fixed frame: max {:.2} m/s, p99 {:.2}, p90 {:.2}, median {:.2}",
            settled_speeds.last().unwrap(),
            at(0.99),
            at(0.90),
            at(0.5),
        );
    }

    /// Silhouette areas of the drawn model, for an air-drag estimate: the union
    /// of its triangles projected head-on (onto the y-z plane) and broadside.
    #[test]
    #[ignore]
    fn ship_silhouette_areas() {
        let mesh = build_mesh();
        let area = |a: usize, b: usize| {
            let cell = 0.02f32;
            let mut covered = std::collections::HashSet::new();
            for tri in mesh.chunks_exact(3) {
                let p: Vec<[f32; 2]> = tri.iter().map(|v| [v.position[a], v.position[b]]).collect();
                let (min_x, max_x) = (p.iter().map(|q| q[0]).fold(f32::MAX, f32::min), p.iter().map(|q| q[0]).fold(f32::MIN, f32::max));
                let (min_y, max_y) = (p.iter().map(|q| q[1]).fold(f32::MAX, f32::min), p.iter().map(|q| q[1]).fold(f32::MIN, f32::max));
                let edge = |u: [f32; 2], v: [f32; 2], w: [f32; 2]| (v[0] - u[0]) * (w[1] - u[1]) - (v[1] - u[1]) * (w[0] - u[0]);
                let mut x = (min_x / cell).floor() * cell;
                while x <= max_x {
                    let mut y = (min_y / cell).floor() * cell;
                    while y <= max_y {
                        let c = [x + 0.5 * cell, y + 0.5 * cell];
                        let (e0, e1, e2) = (edge(p[0], p[1], c), edge(p[1], p[2], c), edge(p[2], p[0], c));
                        if (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0) {
                            covered.insert(((x / cell).round() as i32, (y / cell).round() as i32));
                        }
                        y += cell;
                    }
                    x += cell;
                }
            }
            covered.len() as f32 * cell * cell
        };
        println!("head-on (y-z) silhouette {:.2} m2, broadside (x-z) {:.2} m2, mass {:.0} kg", area(1, 2), area(0, 2), ShipHull::new().mass_kg());
    }

    #[test]
    fn the_float_is_deterministic() {
        let (hull, mut first) = afloat();
        let (_, mut second) = afloat();
        for _ in 0..120 {
            first.advance(&hull, 1.0 / 60.0, still_water(1.5));
            second.advance(&hull, 1.0 / 60.0, still_water(1.5));
        }
        assert_eq!(first.position, second.position);
        assert_eq!(first.orientation, second.orientation);
    }

    #[test]
    fn the_hull_body_stays_in_the_float_envelope_and_the_model_is_detailed() {
        let model = crate::ship_model::build();
        let mesh = build_mesh();
        assert_eq!(mesh.len(), model.hull.len() + model.fittings.len());
        assert_eq!(mesh.len() % 3, 0);
        let triangles = mesh.len() / 3;
        assert!(
            (1_500..=12_000).contains(&triangles),
            "{triangles} triangles is not a detailed galleon"
        );
        // The hull body is what the float, foam and spray are tuned to.
        for vertex in &model.hull {
            let [x, y, z] = vertex.position;
            assert!(x.abs() <= 0.5 * HULL_LENGTH_METERS as f32 + 0.01);
            assert!(y.abs() <= 0.5 * HULL_BEAM_METERS as f32 + 0.01);
            assert!(z >= -(HULL_DRAFT_METERS as f32) - 0.01);
            assert!(z <= (HULL_FREEBOARD_METERS + 9.0 * SHIP_SCALE) as f32);
        }
        // Masts stand tall over the deck and nothing is below the keel.
        let top = mesh.iter().map(|v| v.position[2]).fold(f32::MIN, f32::max);
        let bottom = mesh.iter().map(|v| v.position[2]).fold(f32::MAX, f32::min);
        assert!(top > (30.0 * SHIP_SCALE) as f32, "mast top {top}m");
        assert!(bottom >= -(HULL_DRAFT_METERS as f32) - 0.01, "keel {bottom}m");
        for vertex in &mesh {
            let normal = glam::Vec3::from(vertex.normal);
            assert!((normal.length() - 1.0).abs() < 1.0e-4);
        }
        // The hull body fills its envelope end to end: the drawn transom and
        // stem are where buoyancy, foam and spray put them.
        let aft = model.hull.iter().map(|v| v.position[0]).fold(f32::MAX, f32::min);
        let fore = model.hull.iter().map(|v| v.position[0]).fold(f32::MIN, f32::max);
        let half_length = 0.5 * HULL_LENGTH_METERS as f32;
        assert!((aft + half_length).abs() < 0.01, "transom drawn at {aft}m, not -{half_length}m");
        assert!((fore - half_length).abs() < 0.01, "stem drawn at {fore}m, not {half_length}m");
        // Flat-shaded: three vertices share a normal.
        for triangle in mesh.chunks_exact(3) {
            assert_eq!(triangle[0].normal, triangle[1].normal);
            assert_eq!(triangle[1].normal, triangle[2].normal);
        }
    }

    #[test]
    fn hull_sides_face_outward_and_the_deck_faces_up() {
        let model = crate::ship_model::build();
        // Every hull face below the sheer on the sides points away from the
        // centreline (the bulwarks' inner faces above it point in, by design).
        let mut checked = 0;
        for triangle in model.hull.chunks_exact(3) {
            let normal = glam::Vec3::from(triangle[0].normal);
            let centre = (glam::Vec3::from(triangle[0].position)
                + glam::Vec3::from(triangle[1].position)
                + glam::Vec3::from(triangle[2].position))
                / 3.0;
            let below_deck = centre.z < sheer_height_meters(centre.x as f64 / (0.5 * HULL_LENGTH_METERS)) as f32 - 0.01;
            if below_deck && normal.z.abs() < 0.5 && normal.y.abs() > 0.5 && centre.y.abs() > 0.05 {
                assert!(normal.y * centre.y > 0.0, "side face points inward at {centre:?}");
                checked += 1;
            }
        }
        assert!(checked > 200, "{checked} side faces checked");
    }

    #[test]
    fn a_floating_hull_is_carried_by_the_water_and_not_held_still() {
        // Drag is relative to the water: in a steady 2 m/s current the hull
        // ends up moving with it. Damped toward the planet instead, it stood
        // still while crests swept under it and was thrown clear of them.
        let (hull, mut body) = afloat();
        let east = body.position.normalize().cross(DVec3::Y).normalize();
        let current = |_: DVec3| WaterSample {
            height_meters: 0.0,
            vertical_velocity_meters_per_second: 0.0,
            slope: DVec3::ZERO,
            horizontal_velocity: east * 2.0,
        };
        for _ in 0..(10.0 / super::FIXED_STEP_SECONDS) as usize {
            body.advance(&hull, super::FIXED_STEP_SECONDS, current);
        }
        let along = body.linear_velocity.dot(east);
        assert!((along - 2.0).abs() < 0.05, "hull moves at {along} m/s in a 2 m/s current");

        // Clear of the water nothing drags it: horizontal speed is kept.
        let (hull, mut body) = afloat();
        body.position += body.position.normalize() * 50.0;
        body.linear_velocity = east * 3.0;
        body.advance(&hull, super::FIXED_STEP_SECONDS * 10.0, current);
        assert!((body.linear_velocity.dot(east) - 3.0).abs() < 1.0e-4);
    }

    #[test]
    fn splashes_need_the_water_against_the_hull_side() {
        // Floating at its waterline: full contact, below the deck.
        assert_eq!(super::hull_water_contact(0.0, 0.0), (1.0, 1.0));
        // Hull thrown clear of the water: no contact.
        assert_eq!(super::hull_water_contact(0.0, -2.0 * HULL_DRAFT_METERS).0, 0.0);
        // Buried with the sea over the deck: no contact, and above the deck.
        let buried = super::hull_water_contact(0.0, 2.0 * HULL_FREEBOARD_METERS);
        assert_eq!(buried, (0.0, 0.0));
    }

    #[test]
    fn the_hull_form_tapers_to_a_stem_and_keeps_a_broad_transom() {
        assert!(half_beam_meters(1.0) < 0.01);
        assert!(half_beam_meters(-1.0) > 0.35 * HULL_BEAM_METERS);
        assert!((half_beam_meters(0.0) - 0.5 * HULL_BEAM_METERS).abs() < 1.0e-9);
        // The forefoot rises toward the stem, so the bow can lift over a wave.
        assert!(keel_depth_meters(1.0) < 0.5 * keel_depth_meters(0.0));
        assert!((keel_depth_meters(0.0) - HULL_DRAFT_METERS).abs() < 1.0e-9);
        const {
            assert!(HULL_FREEBOARD_METERS > 0.0);
        }
    }
}
