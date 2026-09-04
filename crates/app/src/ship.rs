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

use crate::planet::PLANET_RADIUS_METERS;
use crate::surface_camera::GRAVITY_METERS_PER_SECOND_SQUARED;

pub const HULL_LENGTH_METERS: f64 = 42.0;
pub const HULL_BEAM_METERS: f64 = 11.0;
/// Keel below the design waterline amidships.
pub const HULL_DRAFT_METERS: f64 = 3.0;
/// Deck above the design waterline amidships.
pub const HULL_FREEBOARD_METERS: f64 = 3.0;

const SEAWATER_DENSITY_KG_PER_CUBIC_METER: f64 = 1025.0;
/// Longitudinal and transverse buoyancy columns. The hull only pitches and
/// rolls with a wave because separate columns see different water heights, so
/// this is the resolution of the float, not just of the volume integral.
const BUOYANCY_STATIONS: usize = 16;
const BUOYANCY_COLUMNS: usize = 5;
/// Vertical drag per square metre of plan area. Chosen for a heave damping
/// ratio near 0.3: enough that a dropped hull settles in a few oscillations
/// rather than ringing, and far short of pinning it to the surface.
const HEAVE_DRAG_KG_PER_SQUARE_METER_SECOND: f64 = 3_100.0;
/// The first metre of immersion fades drag in. A column that switches its drag
/// on at full strength the instant it touches makes a hull chatter along a
/// crest instead of riding it.
const DRAG_IMMERSION_FADE_METERS: f64 = 1.0;
/// Horizontal water resistance, as a fraction of speed shed per second. The
/// hull carries no propulsion, so this is what stops wave impulses walking it
/// across the ocean.
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
const YAW_DAMPING_PER_SECOND: f64 = 0.4;
/// Eddy and bilge-keel roll damping, as a fraction of roll rate shed per
/// second. The buoyancy columns damp heave and pitch well, because those act
/// over the hull's length; roll acts over its beam and comes out badly
/// under-damped, which is the same reason real hulls carry bilge keels. Without
/// this the hull answers a 23-degree sea with a 55-degree knockdown.
const ROLL_DAMPING_PER_SECOND: f64 = 0.9;
/// Metacentric height: the single number that sets how a hull rolls. The mass
/// centre is then placed to produce it, rather than the other way round.
///
/// With mass at the buoyancy centre, as the first cut had it, GM is the whole
/// metacentric radius -- 2.8m on this form, well over a real coaster's -- and
/// the hull is far too stiff to roll, snapping upright in a couple of seconds
/// and reading as welded to the water. Loading it until GM nearly vanished
/// instead let a 23-degree sea knock it down to 55. Small cargo ships run
/// around 0.5 to 1.5m; roll period grows as 1/sqrt(GM).
const METACENTRIC_HEIGHT_METERS: f64 = 0.9;
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
                * (PLANET_RADIUS_METERS + water_height_meters + hull.centre_of_mass_local.z),
            orientation: DQuat::from_mat3(&DMat3::from_cols(forward, port, up)),
            linear_velocity: DVec3::ZERO,
            angular_velocity: DVec3::ZERO,
        }
    }

    /// Altitude of the ship-local origin: the point on the hull that should sit
    /// level with the water when it floats at its design draft.
    pub fn waterline_altitude_meters(&self, hull: &ShipHull) -> f64 {
        (self.position + self.orientation * -hull.centre_of_mass_local).length()
            - PLANET_RADIUS_METERS
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

        for column in &hull.columns {
            let keel_offset = rotation * (column.keel_local - hull.centre_of_mass_local);
            let keel_world = self.position + keel_offset;
            let column_direction = keel_world.normalize();
            let keel_altitude = keel_world.length() - PLANET_RADIUS_METERS;
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
        // rising crest.
        let vertical_velocity = radial * self.linear_velocity.dot(radial);
        let horizontal_velocity = self.linear_velocity - vertical_velocity;
        self.linear_velocity = vertical_velocity
            + horizontal_velocity * (1.0 - SURGE_DAMPING_PER_SECOND * step_seconds).max(0.0);
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

const HULL_BELOW_WATERLINE_COLOUR: [f32; 3] = [0.32, 0.09, 0.07];
const HULL_TOPSIDE_COLOUR: [f32; 3] = [0.10, 0.13, 0.18];
const DECK_COLOUR: [f32; 3] = [0.46, 0.35, 0.22];
const CABIN_COLOUR: [f32; 3] = [0.72, 0.71, 0.68];
const FUNNEL_COLOUR: [f32; 3] = [0.55, 0.16, 0.12];

/// Mesh stations. Coarse on purpose: the facets are the presentation.
const MESH_STATIONS: usize = 10;

fn push_triangle(vertices: &mut Vec<ShipVertex>, a: DVec3, b: DVec3, c: DVec3, colour: [f32; 3]) {
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

fn push_quad(
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

fn push_box(vertices: &mut Vec<ShipVertex>, centre: DVec3, half_extents: DVec3, colour: [f32; 3]) {
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

/// Builds the hull, deck, cabin and funnel as a flat-shaded triangle list in
/// ship-local metres. Faces are wound counter-clockwise from outside.
pub fn build_mesh() -> Vec<ShipVertex> {
    let mut vertices = Vec::new();

    let station = |index: usize| {
        let t = station_parameter(index, MESH_STATIONS - 1) + 1.0 / (MESH_STATIONS - 1) as f64;
        let t = t.clamp(-1.0, 1.0);
        let x = 0.5 * HULL_LENGTH_METERS * t;
        (
            x,
            half_beam_meters(t),
            keel_depth_meters(t),
            sheer_height_meters(t),
        )
    };

    for index in 0..MESH_STATIONS - 1 {
        let (x0, beam0, keel0, sheer0) = station(index);
        let (x1, beam1, keel1, sheer1) = station(index + 1);

        // Bottom, from the keel line out to the chine at half draft.
        let keel_aft = DVec3::new(x0, 0.0, -keel0);
        let keel_fwd = DVec3::new(x1, 0.0, -keel1);
        for side in [1.0, -1.0] {
            let chine_aft = DVec3::new(x0, side * beam0, -0.5 * keel0);
            let chine_fwd = DVec3::new(x1, side * beam1, -0.5 * keel1);
            let sheer_aft = DVec3::new(x0, side * beam0, sheer0);
            let sheer_fwd = DVec3::new(x1, side * beam1, sheer1);
            if side > 0.0 {
                push_quad(
                    &mut vertices,
                    keel_aft,
                    chine_aft,
                    chine_fwd,
                    keel_fwd,
                    HULL_BELOW_WATERLINE_COLOUR,
                );
                push_quad(
                    &mut vertices,
                    chine_aft,
                    sheer_aft,
                    sheer_fwd,
                    chine_fwd,
                    HULL_TOPSIDE_COLOUR,
                );
            } else {
                push_quad(
                    &mut vertices,
                    keel_aft,
                    keel_fwd,
                    chine_fwd,
                    chine_aft,
                    HULL_BELOW_WATERLINE_COLOUR,
                );
                push_quad(
                    &mut vertices,
                    chine_aft,
                    chine_fwd,
                    sheer_fwd,
                    sheer_aft,
                    HULL_TOPSIDE_COLOUR,
                );
            }
        }

        // Deck, closing the two sheer lines across the centreline.
        push_quad(
            &mut vertices,
            DVec3::new(x0, beam0, sheer0),
            DVec3::new(x0, -beam0, sheer0),
            DVec3::new(x1, -beam1, sheer1),
            DVec3::new(x1, beam1, sheer1),
            DECK_COLOUR,
        );
    }

    // Transom, closing the open stern.
    let (x_aft, beam_aft, keel_aft, sheer_aft) = station(0);
    push_quad(
        &mut vertices,
        DVec3::new(x_aft, beam_aft, sheer_aft),
        DVec3::new(x_aft, beam_aft, -0.5 * keel_aft),
        DVec3::new(x_aft, -beam_aft, -0.5 * keel_aft),
        DVec3::new(x_aft, -beam_aft, sheer_aft),
        HULL_TOPSIDE_COLOUR,
    );
    push_triangle(
        &mut vertices,
        DVec3::new(x_aft, beam_aft, -0.5 * keel_aft),
        DVec3::new(x_aft, 0.0, -keel_aft),
        DVec3::new(x_aft, -beam_aft, -0.5 * keel_aft),
        HULL_BELOW_WATERLINE_COLOUR,
    );

    // Superstructure: a two-tier deckhouse set aft, and a funnel.
    push_box(
        &mut vertices,
        DVec3::new(-6.0, 0.0, HULL_FREEBOARD_METERS + 1.6),
        DVec3::new(6.0, 3.6, 1.6),
        CABIN_COLOUR,
    );
    push_box(
        &mut vertices,
        DVec3::new(-8.0, 0.0, HULL_FREEBOARD_METERS + 4.0),
        DVec3::new(3.4, 2.8, 1.0),
        CABIN_COLOUR,
    );
    push_box(
        &mut vertices,
        DVec3::new(-10.5, 0.0, HULL_FREEBOARD_METERS + 6.2),
        DVec3::new(1.3, 1.3, 1.4),
        FUNNEL_COLOUR,
    );

    vertices
}

#[cfg(test)]
mod tests {
    use glam::{DQuat, DVec3};

    use crate::ocean;

    use super::{
        HULL_BEAM_METERS, HULL_DRAFT_METERS, HULL_FREEBOARD_METERS, HULL_LENGTH_METERS, ShipBody,
        ShipHull, WaterSample, build_mesh, half_beam_meters, keel_depth_meters,
    };
    use crate::planet::PLANET_RADIUS_METERS;

    const START_DIRECTION: DVec3 = DVec3::new(0.838, 0.502, 0.2125);

    fn still_water(height_meters: f64) -> impl Fn(DVec3) -> WaterSample {
        move |_| WaterSample {
            height_meters,
            vertical_velocity_meters_per_second: 0.0,
            slope: DVec3::ZERO,
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
        // tens of thousands. This is the sanity bound on the whole float.
        let volume = hull.displaced_volume_cubic_meters();
        assert!(
            (600.0..1200.0).contains(&volume),
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
            * (PLANET_RADIUS_METERS + 6.0 + hull.centre_of_mass_local().z);
        let mut extremes = 0;
        let mut previous_altitude = body.waterline_altitude_meters(&hull);
        let mut rising = false;
        for _ in 0..600 {
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
        assert!(body.waterline_altitude_meters(&hull).abs() < 0.05);
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
            let along = direction.dot(forward) * PLANET_RADIUS_METERS;
            WaterSample {
                height_meters: slope * along,
                vertical_velocity_meters_per_second: 0.0,
                slope: DVec3::ZERO,
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
        assert!(2.0 * AMPLITUDE_METERS / WAVELENGTH_METERS < 1.0 / 7.0);
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
                    direction.dot(DVec3::new(0.6, 0.5, 0.62).normalize()) * PLANET_RADIUS_METERS;
                WaterSample {
                    height_meters: AMPLITUDE_METERS * (wave_number * phase + 1.6 * elapsed).sin(),
                    vertical_velocity_meters_per_second: AMPLITUDE_METERS
                        * 1.6
                        * (wave_number * phase + 1.6 * elapsed).cos(),
                    slope: DVec3::ZERO,
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
        assert!(
            (0.5..=1.5).contains(&hull.metacentric_height_meters()),
            "GM {} m is outside the range small cargo ships run",
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
        assert!(
            reversals(&roll) > 8,
            "roll reversed {} times",
            reversals(&roll)
        );
        assert!(
            reversals(&pitch) > 8,
            "pitch reversed {} times",
            reversals(&pitch)
        );

        // And the wave-slope forcing that drives yaw must not walk the hull
        // out of the scene while it does so.
        let drift_meters = (body.position.normalize() - START_DIRECTION.normalize()).length()
            * PLANET_RADIUS_METERS;
        assert!(drift_meters < 25.0, "hull drifted {drift_meters} m in 50s");
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
        };
        single.advance(&hull, super::FIXED_STEP_SECONDS * 8.0, water);
        for _ in 0..8 {
            chunked.advance(&hull, super::FIXED_STEP_SECONDS, water);
        }
        assert_eq!(single.position, chunked.position);
        assert_eq!(single.orientation, chunked.orientation);
        assert_eq!(single.linear_velocity, chunked.linear_velocity);
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
    fn the_mesh_is_low_poly_closed_and_within_the_hull_envelope() {
        let mesh = build_mesh();
        assert_eq!(mesh.len() % 3, 0);
        let triangles = mesh.len() / 3;
        // Low poly is a requirement here, not an accident of the generator.
        assert!(
            (80..=400).contains(&triangles),
            "{triangles} triangles is not a low-poly hull"
        );
        for vertex in &mesh {
            let [x, y, z] = vertex.position;
            assert!(x.abs() <= 0.5 * HULL_LENGTH_METERS as f32 + 0.01);
            assert!(y.abs() <= 0.5 * HULL_BEAM_METERS as f32 + 0.01);
            assert!(z >= -(HULL_DRAFT_METERS as f32) - 0.01);
            assert!(z <= (HULL_FREEBOARD_METERS + 9.0) as f32);
            let normal = glam::Vec3::from(vertex.normal);
            assert!((normal.length() - 1.0).abs() < 1.0e-4);
        }
        // Every triangle is flat-shaded, so its three vertices share a normal.
        for triangle in mesh.chunks_exact(3) {
            assert_eq!(triangle[0].normal, triangle[1].normal);
            assert_eq!(triangle[1].normal, triangle[2].normal);
        }
    }

    #[test]
    fn the_hull_form_tapers_to_a_stem_and_keeps_a_broad_transom() {
        assert!(half_beam_meters(1.0) < 0.01);
        assert!(half_beam_meters(-1.0) > 0.35 * HULL_BEAM_METERS);
        assert!((half_beam_meters(0.0) - 0.5 * HULL_BEAM_METERS).abs() < 1.0e-9);
        // The forefoot rises toward the stem, so the bow can lift over a wave.
        assert!(keel_depth_meters(1.0) < 0.5 * keel_depth_meters(0.0));
        assert!((keel_depth_meters(0.0) - HULL_DRAFT_METERS).abs() < 1.0e-9);
        assert!(HULL_FREEBOARD_METERS > 0.0);
    }
}
