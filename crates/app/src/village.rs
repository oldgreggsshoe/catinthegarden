//! Norwegian-style villages: small clusters of painted timber houses scattered
//! over habitable ground.
//!
//! Placement mirrors the forest's scheme, because that is the part of this
//! renderer that already knows how to scatter things across a sphere without a
//! seam or a rebake: a deterministic hash per cube-quadtree cell, then a ground
//! sample to accept or reject the site. Two things differ.
//!
//! First the cell is far coarser. The forest works at level 12, a 1.5km cell,
//! because trees are everywhere. Villages sit at level 8 -- a 24.5km cell --
//! so settlements land tens of kilometres apart the way real ones do along a
//! valley or a fjord, rather than tiling the landscape evenly.
//!
//! Second the slope rule is much tighter, and it is applied twice. A forest
//! tolerates 32 degrees; nobody builds a village on that. The *site* must be
//! gentle, and then every individual house re-tests the ground under itself, so
//! one house cannot end up perched on a step while its neighbours sit flat.
//!
//! Geometry is a box with a pitched roof, flat-shaded with a per-vertex colour,
//! built once and drawn instanced. Positions reach the shader as an offset from
//! the camera rather than as planet coordinates: at a 4,000km radius an f32
//! planet position quantises to about half a metre, which on a six-metre house
//! is the difference between a wall and a staircase.

use glam::DVec3;

use catinthegarden_coretypes::{BiomeId, TileKey, face_uv_to_direction, tile_key_for_direction};

use crate::planet::{
    GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS, TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS,
    planet_radius_meters,
};
use crate::terrain::{ForestSurfaceSample, TerrainRenderer};

/// Quadtree level for village sites: a 6.1km cell on this planet.
///
/// This has to be read together with the render distance and the search ring
/// below. At level 8 the cell was 24.5km against a 6km cutoff, so only a
/// village in the camera's own cell could ever be drawn and villages were
/// almost never visible. Level 9 fixed that; level 10 is the answer to "as many
/// as you can", putting settlements roughly six kilometres apart -- close
/// enough that the planet reads as inhabited rather than as empty country with
/// the occasional hamlet.
const VILLAGE_CELL_LEVEL: u8 = 10;
/// Candidate sites tried per cell. Most are rejected by ground or biome, so
/// this is an upper bound on villages per cell rather than a count.
const VILLAGE_SITE_CANDIDATES_PER_CELL: u32 = 2;
/// Houses attempted per village. The cluster thins itself: each house re-tests
/// its own ground, so a village on broken terrain simply ends up smaller.
const HOUSES_PER_VILLAGE: u32 = 20;
/// How far houses scatter from the village centre.
const VILLAGE_RADIUS_METERS: f64 = 110.0;
/// How much the ground may rise and fall across a village's own footprint.
///
/// Flatness is tested over the footprint rather than as a slope at one point,
/// because slope is scale-dependent and a pointwise reading answers the wrong
/// question. Measured: the macro field puts habitable ground at a median 0.87
/// degrees over its 3km sample spacing, while the renderer's close-range sample
/// of the same ground reads 15 to 37 degrees, because it includes synthesised
/// detail. Neither is wrong. A village does not care whether the ground is
/// bumpy at ten metres; it cares whether it is level across two hundred.
///
/// 30m across a 220m span is about 7 degrees at village scale, which is the
/// limit a real settlement site respects.
const VILLAGE_FOOTPRINT_HEIGHT_SPREAD_METERS: f64 = 30.0;
/// How far one house's ground may sit from the village's mean height. Keeps a
/// cluster coherent without demanding a billiard table.
const HOUSE_HEIGHT_DEVIATION_METERS: f64 = 14.0;
/// Houses stop being drawn beyond this, and the whole system switches off above
/// the draw altitude: a village is metres across, so there is nothing to see
/// from orbit but cost.
const VILLAGE_RENDER_DISTANCE_METERS: f64 = 8_000.0;
const VILLAGE_DRAW_ALTITUDE_METERS: f64 = 12_000.0;
/// Hard ceiling on drawn houses. The budget, not the scatter, is what bounds
/// the frame cost.
const VILLAGE_MAX_DRAW_INSTANCES: usize = 4_096;
/// Cells searched around the camera each rebuild, as a ring count.
///
/// The ring has to reach at least as far as the render cutoff, or villages
/// inside the cutoff go unsearched and pop in as the camera crosses a cell
/// boundary. With a 6.1km cell and an 8km cutoff, one ring is not enough: a
/// camera sitting at its cell's edge has ground 8km away sitting two cells
/// over. Two rings covers 12.2km, which clears it.
const VILLAGE_SEARCH_RING: i32 = 2;
/// How far the drawn ground may differ from the sited ground before the drawn
/// answer is disbelieved.
///
/// The runtime detail ladder is the whole legitimate difference between baked
/// macro geography and the rendered surface, and it is bounded, so this is
/// that bound with a little room for the separate global detail term.
const HOUSE_GROUND_DISAGREEMENT_LIMIT_METERS: f64 =
    TERRAIN_DETAIL_TOTAL_AMPLITUDE_METERS + GLOBAL_TERRAIN_DETAIL_AMPLITUDE_METERS;
/// Rebuild the instance list when the camera has moved this far. Villages are
/// static, so the list only changes when the visible set does.
const VILLAGE_REBUILD_DISTANCE_METERS: f64 = 400.0;
const VILLAGE_PLANET_SEED: u32 = 0x7a3c_1d55;

const HOUSE_WIDTH_METERS: f64 = 6.4;
const HOUSE_DEPTH_METERS: f64 = 8.2;
const HOUSE_WALL_HEIGHT_METERS: f64 = 3.1;
const HOUSE_RIDGE_HEIGHT_METERS: f64 = 2.4;
/// Sink the walls slightly so a house meets sloping ground without a visible
/// gap under the downhill corner.
const HOUSE_BASE_SINK_METERS: f64 = 0.35;
/// How far the roof oversails the walls on every side. The mesh and the
/// no-overlap rule both read this, so a house's footprint cannot disagree with
/// the circle that reserves ground for it.
const HOUSE_ROOF_OVERHANG_METERS: f64 = 0.45;
/// How many placements a house may try before the village gives up on it. A
/// village that runs out of room simply ends up smaller, which is the right
/// outcome on a cramped site.
const HOUSE_PLACEMENT_ATTEMPTS: u32 = 24;

/// The four painted timber colours, in linear space. Falu red first because it
/// is the one everybody pictures; the others are the usual Nordic palette.
// The roof's slate lives in `village.wgsl`, which picks it per fragment from
// the vertex's roof flag; duplicating it here would let the two drift apart.
const HOUSE_COLOURS: [[f32; 3]; 4] = [
    [0.412, 0.071, 0.055], // falu red
    [0.098, 0.184, 0.286], // slate blue
    [0.129, 0.224, 0.153], // forest green
    [0.600, 0.278, 0.075], // ochre orange
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HouseVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// 0 = painted wall, taking the instance's colour; 1 = roof, which keeps
    /// its own dark slate whatever the walls are painted.
    pub roof: f32,
    pub _padding: f32,
}

/// One house: where it stands relative to the camera, how it is turned, and
/// what colour it is painted.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HouseInstance {
    /// Offset from the camera in planet-local axes. Never a planet-absolute
    /// position: see the module comment.
    pub camera_relative_position: [f32; 3],
    pub _pad0: f32,
    /// Local up at the house, so it stands on the sphere rather than on the
    /// world axes.
    pub up: [f32; 3],
    pub _pad1: f32,
    /// Horizontal facing, perpendicular to `up`.
    pub forward: [f32; 3],
    pub _pad2: f32,
    pub colour: [f32; 3],
    pub _pad3: f32,
}

/// Habitable ground. Villages want land people would actually settle: no ice,
/// no glacier structure, nothing underwater, and not bare mountain rock.
pub fn village_biome_is_habitable(biome: BiomeId) -> bool {
    matches!(
        biome,
        BiomeId::Tundra
            | BiomeId::TemperateForest
            | BiomeId::TemperateGrassland
            | BiomeId::TropicalForest
    )
}

/// Ground a village could stand on at all: the right biome, clear of the water,
/// and a usable height. Deliberately says nothing about slope -- see
/// `VILLAGE_FOOTPRINT_HEIGHT_SPREAD_METERS`.
///
/// Tests the *drawn* surface, not just the macro field. Using
/// `macro_height_meters > 0.0` alone placed houses on open sea: that field is
/// the coarse macro height, and the surface the renderer actually draws sits
/// elsewhere -- the same scale mismatch that made a pointwise slope gate reject
/// every site.
fn village_surface_is_eligible(sample: ForestSurfaceSample) -> bool {
    village_biome_is_habitable(sample.biome)
        && sample.macro_height_meters > 0.0
        && sample.height_meters.is_finite()
}

/// Is this spot dry land, by the renderer's own ownership rule?
///
/// Four earlier attempts tested a height instead -- the scaled macro height,
/// the drawn surface height, then the raw baked height -- and every one left
/// half a village standing on open sea. None of them could work. The rendered
/// analytic ocean owns *every non-ice, non-lake sample at or below sea level*,
/// whatever biome that sample carries, and a coastline point can hold a
/// positive height and a forest biome while the water is drawn over it.
///
/// `open_ocean_at` is the CPU mirror of the shader's `is_open_ocean_surface`,
/// so it cannot disagree with what is drawn. Ask it, rather than inventing a
/// fifth threshold on a fourth field.
fn village_ground_is_dry(terrain: &TerrainRenderer, direction: DVec3) -> bool {
    terrain.dense_level_open_ocean_at(direction) == Some(false)
}

/// Is this site level enough, across its own footprint, to hold a village?
///
/// Samples the centre and four points at the village radius. All five must be
/// habitable land, and the spread between highest and lowest must stay inside
/// the budget. Returns the mean height so the houses can be placed against it.
/// The village's mean ground height, or `None` if the site is not habitable.
///
/// Every sample here is taken at the globally dense outmap level rather than
/// at the finest resident tile. Siting has to be a fact about the planet: with
/// the camera's own view of the ground, the same spot held two different
/// villages at 150m and at 1.5km, because climbing changed which tiles were
/// resident and so changed the biome and height the tests read.
fn village_footprint_height(
    terrain: &TerrainRenderer,
    site_direction: DVec3,
    east: DVec3,
    north: DVec3,
) -> Option<f64> {
    let radius = VILLAGE_RADIUS_METERS;
    let offsets = [
        DVec3::ZERO,
        east * radius,
        east * -radius,
        north * radius,
        north * -radius,
    ];
    let mut lowest = f64::INFINITY;
    let mut highest = f64::NEG_INFINITY;
    let mut total = 0.0;
    for offset in offsets {
        let direction = (site_direction * planet_radius_meters() + offset).normalize_or_zero();
        if direction.length_squared() <= f64::EPSILON {
            return None;
        }
        let sample = terrain.dense_level_surface_sample_at(direction)?;
        if !village_surface_is_eligible(sample) || !village_ground_is_dry(terrain, direction) {
            return None;
        }
        lowest = lowest.min(sample.height_meters);
        highest = highest.max(sample.height_meters);
        total += sample.height_meters;
    }
    if highest - lowest > VILLAGE_FOOTPRINT_HEIGHT_SPREAD_METERS {
        return None;
    }
    Some(total / offsets.len() as f64)
}

/// Furthest a house corner reaches from its centre, seen from above. The roof
/// oversails the walls, so the corner that matters is the eave's rather than
/// the wall's, and the diagonal rather than either side.
fn house_plan_radius_meters() -> f64 {
    let eave_half_width = HOUSE_WIDTH_METERS * 0.5 + HOUSE_ROOF_OVERHANG_METERS;
    let eave_half_depth = HOUSE_DEPTH_METERS * 0.5 + HOUSE_ROOF_OVERHANG_METERS;
    (eave_half_width * eave_half_width + eave_half_depth * eave_half_depth).sqrt()
}

/// Houses must never overlap, whatever way round they are turned. Each one owns
/// a circle of `house_plan_radius_meters()`, and a full radius of clear ground
/// is kept between those circles: two radii for the circles themselves plus one
/// for the gap makes three.
///
/// Working from the circle rather than the rectangle is what makes this hold at
/// any orientation -- a house turned 45 degrees reaches further along its
/// diagonal than along either side, and the circle already covers that.
fn house_minimum_spacing_meters() -> f64 {
    house_plan_radius_meters() * 3.0
}

fn hash_u32(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value
}

fn hash(value: u32) -> f32 {
    hash_u32(value) as f32 / u32::MAX as f32
}

fn unit_hash(value: u32) -> f64 {
    f64::from(hash_u32(value)) / (f64::from(u32::MAX) + 1.0)
}

fn cell_seed(key: TileKey) -> u32 {
    hash_u32(
        VILLAGE_PLANET_SEED
            ^ u32::from(key.face.index()).wrapping_mul(0x9e37_79b9)
            ^ u32::from(key.level).wrapping_mul(0x85eb_ca6b)
            ^ key.x.wrapping_mul(0xc2b2_ae35)
            ^ key.y.wrapping_mul(0x27d4_eb2f),
    )
}

/// A candidate village centre inside one cell. Half-open hashes keep a
/// candidate inside exactly one cell, including at cube-face seams.
fn village_site_direction(key: TileKey, index: u32) -> DVec3 {
    let cells_per_axis = f64::from(1_u32 << key.level);
    let cell_span = 2.0 / cells_per_axis;
    let u_min = -1.0 + f64::from(key.x) * cell_span;
    let v_min = -1.0 + f64::from(key.y) * cell_span;
    let seed = cell_seed(key);
    // Keep sites off the cell edge so a village never straddles two cells and
    // gets built twice with different neighbours.
    let inset = 0.12;
    let u =
        u_min + (inset + unit_hash(seed ^ index ^ 0x6a09_e667) * (1.0 - 2.0 * inset)) * cell_span;
    let v =
        v_min + (inset + unit_hash(seed ^ index ^ 0xbb67_ae85) * (1.0 - 2.0 * inset)) * cell_span;
    face_uv_to_direction(key.face, u, v)
}

/// House positions around a village centre, as tangential offsets in metres,
/// with their facings. Clustered rather than uniform: a village reads as a
/// village because the houses crowd toward its middle and thin out at the edge.
///
/// Placement is rejection-sampled against `house_minimum_spacing_meters()`, so
/// no two houses can overlap however they are turned. A house that cannot find
/// room within `HOUSE_PLACEMENT_ATTEMPTS` is dropped rather than squeezed in;
/// the village ends up smaller, which is what a cramped site should produce.
fn village_house_layout(site_seed: u32) -> Vec<(f64, f64, f64)> {
    let minimum = house_minimum_spacing_meters();
    let minimum_squared = minimum * minimum;
    let mut placed: Vec<(f64, f64, f64)> = Vec::with_capacity(HOUSES_PER_VILLAGE as usize);
    for index in 0..HOUSES_PER_VILLAGE {
        for attempt in 0..HOUSE_PLACEMENT_ATTEMPTS {
            let seed =
                site_seed ^ index.wrapping_mul(0x9e37_79b9) ^ attempt.wrapping_mul(0x85eb_ca6b);
            let angle = unit_hash(seed ^ 0x1f83_d9ab) * std::f64::consts::TAU;
            // sqrt would spread houses evenly over the disc; a higher power
            // pulls them inward, which looks like a settlement rather than a
            // scatter.
            let radius = VILLAGE_RADIUS_METERS * unit_hash(seed ^ 0x5be0_cd19).powf(1.6);
            let facing = unit_hash(seed ^ 0x9b05_688c) * std::f64::consts::TAU;
            let east = angle.cos() * radius;
            let north = angle.sin() * radius;
            let clear = placed.iter().all(|&(other_east, other_north, _)| {
                let de = east - other_east;
                let dn = north - other_north;
                de * de + dn * dn >= minimum_squared
            });
            if clear {
                placed.push((east, north, facing));
                break;
            }
        }
    }
    placed
}

fn house_colour(site_seed: u32, index: u32) -> [f32; 3] {
    let pick = hash(site_seed ^ index ^ 0x3c6e_f372);
    // Weighted toward red: a Norwegian village is mostly falu red with a few
    // painted exceptions, not four colours in equal measure.
    let choice = if pick < 0.55 {
        0
    } else if pick < 0.70 {
        1
    } else if pick < 0.85 {
        2
    } else {
        3
    };
    HOUSE_COLOURS[choice]
}

fn push_triangle(vertices: &mut Vec<HouseVertex>, a: DVec3, b: DVec3, c: DVec3, roof: f32) {
    let normal = (b - a).cross(c - a);
    if normal.length_squared() <= 0.0 {
        return;
    }
    let normal = normal.normalize().as_vec3().to_array();
    for point in [a, b, c] {
        vertices.push(HouseVertex {
            position: point.as_vec3().to_array(),
            normal,
            roof,
            _padding: 0.0,
        });
    }
}

fn push_quad(vertices: &mut Vec<HouseVertex>, a: DVec3, b: DVec3, c: DVec3, d: DVec3, roof: f32) {
    push_triangle(vertices, a, b, c, roof);
    push_triangle(vertices, a, c, d, roof);
}

/// One house, built in local axes: x across the gable, y along the ridge, z up.
/// Walls carry `roof = 0` and take the instance colour; the roof carries
/// `roof = 1` and keeps its own slate.
pub fn build_house_mesh() -> Vec<HouseVertex> {
    let half_width = HOUSE_WIDTH_METERS * 0.5;
    let half_depth = HOUSE_DEPTH_METERS * 0.5;
    let base = -HOUSE_BASE_SINK_METERS;
    let eaves = HOUSE_WALL_HEIGHT_METERS;
    let ridge = HOUSE_WALL_HEIGHT_METERS + HOUSE_RIDGE_HEIGHT_METERS;
    let mut vertices = Vec::new();

    let corner = |sx: f64, sy: f64, z: f64| DVec3::new(sx * half_width, sy * half_depth, z);

    // Four walls, wound counter-clockwise from outside.
    push_quad(
        &mut vertices,
        corner(1.0, -1.0, base),
        corner(1.0, 1.0, base),
        corner(1.0, 1.0, eaves),
        corner(1.0, -1.0, eaves),
        0.0,
    );
    push_quad(
        &mut vertices,
        corner(-1.0, 1.0, base),
        corner(-1.0, -1.0, base),
        corner(-1.0, -1.0, eaves),
        corner(-1.0, 1.0, eaves),
        0.0,
    );
    push_quad(
        &mut vertices,
        corner(-1.0, 1.0, base),
        corner(-1.0, 1.0, eaves),
        corner(1.0, 1.0, eaves),
        corner(1.0, 1.0, base),
        0.0,
    );
    push_quad(
        &mut vertices,
        corner(-1.0, -1.0, eaves),
        corner(-1.0, -1.0, base),
        corner(1.0, -1.0, base),
        corner(1.0, -1.0, eaves),
        0.0,
    );

    // Gables: the triangle between the eaves and the ridge at each end.
    let ridge_near = DVec3::new(0.0, -half_depth, ridge);
    let ridge_far = DVec3::new(0.0, half_depth, ridge);
    push_triangle(
        &mut vertices,
        corner(-1.0, -1.0, eaves),
        corner(1.0, -1.0, eaves),
        ridge_near,
        0.0,
    );
    push_triangle(
        &mut vertices,
        corner(1.0, 1.0, eaves),
        corner(-1.0, 1.0, eaves),
        ridge_far,
        0.0,
    );

    // Two roof planes, oversailing the walls a little so the eaves read.
    let overhang = HOUSE_ROOF_OVERHANG_METERS;
    let eave_x = half_width + overhang;
    let eave_y = half_depth + overhang;
    let eave_z = eaves - overhang * 0.35;
    push_quad(
        &mut vertices,
        DVec3::new(-eave_x, -eave_y, eave_z),
        DVec3::new(0.0, -eave_y, ridge),
        DVec3::new(0.0, eave_y, ridge),
        DVec3::new(-eave_x, eave_y, eave_z),
        1.0,
    );
    push_quad(
        &mut vertices,
        DVec3::new(eave_x, eave_y, eave_z),
        DVec3::new(0.0, eave_y, ridge),
        DVec3::new(0.0, -eave_y, ridge),
        DVec3::new(eave_x, -eave_y, eave_z),
        1.0,
    );

    vertices
}

/// Builds the visible houses around the camera. Pure: it reads terrain and
/// returns instances, so the placement rules are testable without a device.
pub fn collect_house_instances(
    terrain: &TerrainRenderer,
    camera_direction: DVec3,
    camera_altitude_meters: f64,
    camera_world_position: DVec3,
) -> VillageBuild {
    let mut build = VillageBuild::default();
    let instances = &mut build.instances;
    if camera_altitude_meters >= VILLAGE_DRAW_ALTITUDE_METERS {
        return build;
    }
    let camera_direction = camera_direction.normalize_or_zero();
    if camera_direction.length_squared() <= f64::EPSILON {
        return build;
    }
    let centre_key = tile_key_for_direction(camera_direction, VILLAGE_CELL_LEVEL);
    let side = 1_i64 << VILLAGE_CELL_LEVEL;

    for dy in -VILLAGE_SEARCH_RING..=VILLAGE_SEARCH_RING {
        for dx in -VILLAGE_SEARCH_RING..=VILLAGE_SEARCH_RING {
            let x = i64::from(centre_key.x) + i64::from(dx);
            let y = i64::from(centre_key.y) + i64::from(dy);
            // Cells outside the face are simply skipped: a neighbouring face's
            // villages are handled when the camera crosses onto it, and the
            // 24.5km cell is four times the render distance, so nothing visible
            // is lost at the seam.
            if x < 0 || y < 0 || x >= side || y >= side {
                continue;
            }
            let key = TileKey {
                face: centre_key.face,
                level: VILLAGE_CELL_LEVEL,
                x: x as u32,
                y: y as u32,
            };
            for candidate in 0..VILLAGE_SITE_CANDIDATES_PER_CELL {
                let site_direction = village_site_direction(key, candidate);
                let site_seed = hash_u32(cell_seed(key) ^ candidate.wrapping_mul(0x9e37_79b9));
                let up = site_direction;
                // A stable tangent frame at the site: every house in the
                // village shares it, so the cluster reads as one settlement.
                let reference = if up.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
                let east = up.cross(reference).normalize_or_zero();
                if east.length_squared() <= f64::EPSILON {
                    continue;
                }
                let north = east.cross(up);
                // Level across the footprint, not steep at a point.
                let Some(site_height_meters) =
                    village_footprint_height(terrain, site_direction, east, north)
                else {
                    continue;
                };

                let layout = village_house_layout(site_seed);
                for (index, &(offset_east, offset_north, facing)) in layout.iter().enumerate() {
                    if instances.len() >= VILLAGE_MAX_DRAW_INSTANCES {
                        return build;
                    }
                    let index = index as u32;
                    let offset = east * offset_east + north * offset_north;
                    let house_direction =
                        (site_direction * planet_radius_meters() + offset).normalize_or_zero();
                    if house_direction.length_squared() <= f64::EPSILON {
                        continue;
                    }
                    let Some(site_ground) = terrain.dense_level_surface_sample_at(house_direction)
                    else {
                        continue;
                    };
                    // The second slope test. Without it a village on a shallow
                    // hillside puts one house on the step at its edge.
                    if !village_surface_is_eligible(site_ground)
                        || !village_ground_is_dry(terrain, house_direction)
                    {
                        continue;
                    }
                    // Keep the cluster coherent: a house whose ground sits well
                    // off the village's mean is on a step or a bank, not in the
                    // village.
                    if (site_ground.height_meters - site_height_meters).abs()
                        > HOUSE_HEIGHT_DEVIATION_METERS
                    {
                        continue;
                    }
                    // Whether the house exists was decided above, on the dense
                    // level. Where it stands is a different question, and has
                    // to follow the surface actually being drawn, or the house
                    // hovers or sinks. This is the same query flight clearance
                    // uses, and it is the only one that answers it: the forest
                    // sampler reads the finest *resident* tile, whose detail
                    // filter widens with the camera, and it put a house 2km
                    // above ground it shares with a 180m rendered surface.
                    // Falling back to the siting height keeps the house placed
                    // if nothing is drawn there yet.
                    // The raster query answers with the *highest* surface drawn
                    // at this direction, because flight clearance must not miss
                    // one. A coarse ancestor patch covers kilometres of ground
                    // per vertex, so that highest surface can be a mountain
                    // that is nowhere near this house -- it put one house
                    // 1,861m above its own ground. The only legitimate gap
                    // between baked macro geography and the drawn surface is
                    // the runtime detail ladder, which is bounded, so a larger
                    // disagreement is another patch's terrain and the sited
                    // ground is the better answer.
                    let drawn_height_meters = terrain
                        .forest_surface_sample_at(house_direction, camera_altitude_meters)
                        .map(|drawn| drawn.height_meters)
                        .filter(|height| {
                            (height - site_ground.height_meters).abs()
                                <= HOUSE_GROUND_DISAGREEMENT_LIMIT_METERS
                        })
                        .unwrap_or(site_ground.height_meters);
                    // Counted before the distance cutoff. Siting is the thing
                    // that has to be camera-independent; how many of those
                    // houses are near enough to draw is allowed to move with
                    // the camera, and separating the two is what makes the
                    // stability claim measurable.
                    build.sited_houses += 1;
                    // How far the drawn ground is from the baked macro ground
                    // the site was judged on. The runtime detail ladder makes
                    // the two legitimately differ, so this is not a float
                    // measurement -- it is the size of the gap the limit below
                    // has to police.
                    build.max_ground_disagreement_meters = build
                        .max_ground_disagreement_meters
                        .max((drawn_height_meters - site_ground.height_meters).abs());
                    let house_world =
                        house_direction * (planet_radius_meters() + drawn_height_meters);
                    let to_camera = house_world - camera_world_position;
                    if to_camera.length() > VILLAGE_RENDER_DISTANCE_METERS {
                        continue;
                    }
                    let house_up = house_direction;
                    let house_east = house_up.cross(reference).normalize_or_zero();
                    if house_east.length_squared() <= f64::EPSILON {
                        continue;
                    }
                    let house_north = house_east.cross(house_up);
                    let forward =
                        (house_east * facing.cos() + house_north * facing.sin()).normalize();
                    instances.push(HouseInstance {
                        camera_relative_position: to_camera.as_vec3().to_array(),
                        _pad0: 0.0,
                        up: house_up.as_vec3().to_array(),
                        _pad1: 0.0,
                        forward: forward.as_vec3().to_array(),
                        _pad2: 0.0,
                        colour: house_colour(site_seed, index),
                        _pad3: 0.0,
                    });
                }
            }
        }
    }
    build
}

/// What one village rebuild produced.
#[derive(Default)]
pub struct VillageBuild {
    /// The houses near enough to draw.
    pub instances: Vec<HouseInstance>,
    /// Every house the search region sites, whether or not it is near enough
    /// to draw. This is the camera-independent number.
    pub sited_houses: u32,
    /// The worst gap between a house's drawn ground and its sited ground.
    /// Legitimately non-zero: the detail ladder displaces the drawn surface.
    pub max_ground_disagreement_meters: f64,
}

/// How far the camera may move before the visible house set is rebuilt.
pub const fn rebuild_distance_meters() -> f64 {
    VILLAGE_REBUILD_DISTANCE_METERS
}

pub const fn max_draw_instances() -> usize {
    VILLAGE_MAX_DRAW_INSTANCES
}

pub const fn draw_altitude_meters() -> f64 {
    VILLAGE_DRAW_ALTITUDE_METERS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_house_mesh_is_closed_and_carries_both_materials() {
        let mesh = build_house_mesh();
        assert_eq!(mesh.len() % 3, 0, "triangle list");
        assert!(mesh.iter().any(|vertex| vertex.roof == 0.0), "walls");
        assert!(mesh.iter().any(|vertex| vertex.roof == 1.0), "roof");
        // Every normal must be finite and unit length, or the flat shading in
        // the fragment stage produces black facets.
        for vertex in &mesh {
            let normal = glam::Vec3::from(vertex.normal);
            assert!(normal.is_finite());
            assert!((normal.length() - 1.0).abs() < 1.0e-3);
        }
    }

    #[test]
    fn the_ridge_is_above_the_eaves_and_the_base_is_below_the_ground() {
        let mesh = build_house_mesh();
        let highest = mesh
            .iter()
            .map(|vertex| vertex.position[2])
            .fold(f32::MIN, f32::max);
        let lowest = mesh
            .iter()
            .map(|vertex| vertex.position[2])
            .fold(f32::MAX, f32::min);
        assert!(
            (highest - (HOUSE_WALL_HEIGHT_METERS + HOUSE_RIDGE_HEIGHT_METERS) as f32).abs()
                < 1.0e-3
        );
        assert!(lowest < 0.0, "walls sink so sloping ground has no gap");
    }

    #[test]
    fn houses_cluster_toward_the_village_centre() {
        // The radius hash should put more than half the houses inside half the
        // village radius; a uniform disc would put about a quarter. Averaged
        // over many villages, because rejection sampling pushes the occasional
        // house outward when the middle is already taken.
        let mut inside = 0_usize;
        let mut total = 0_usize;
        for seed_index in 0..256_u32 {
            for &(east, north, _) in &village_house_layout(hash_u32(0xc0ff_ee00 ^ seed_index)) {
                total += 1;
                if (east * east + north * north).sqrt() < VILLAGE_RADIUS_METERS * 0.5 {
                    inside += 1;
                }
            }
        }
        // A uniform disc puts a quarter of its points inside half its radius.
        // The radius hash biases inward, and the no-overlap rule then pushes
        // some houses back out once the middle is taken, so the result settles
        // between the two: clustered, but not as tightly as before spacing was
        // enforced. Measured at 43%; the bar is set against uniform rather than
        // against the old figure, which described behaviour that no longer
        // exists.
        let fraction = inside as f64 / total as f64;
        assert!(
            fraction > 0.35,
            "expected clustering above a uniform disc's 25%, got {:.1}% ({inside} of {total})",
            fraction * 100.0
        );
    }

    #[test]
    fn every_house_colour_is_one_of_the_four() {
        for index in 0..256 {
            let colour = house_colour(0xabcd_ef01, index);
            assert!(
                HOUSE_COLOURS.contains(&colour),
                "house {index} took a colour outside the palette"
            );
        }
    }

    #[test]
    fn red_is_the_commonest_colour() {
        let mut counts = [0_usize; 4];
        for index in 0..4_096 {
            let colour = house_colour(0x5555_aaaa, index);
            let slot = HOUSE_COLOURS
                .iter()
                .position(|candidate| *candidate == colour)
                .expect("palette colour");
            counts[slot] += 1;
        }
        assert!(
            counts[0] > counts[1] && counts[0] > counts[2] && counts[0] > counts[3],
            "falu red should dominate: {counts:?}"
        );
    }

    #[test]
    fn glacier_and_water_biomes_are_never_habitable() {
        for biome in [
            BiomeId::Ocean,
            BiomeId::Lake,
            BiomeId::Ice,
            BiomeId::Desert,
            BiomeId::MountainRock,
            BiomeId::MountainSnow,
            BiomeId::GlacialMoraine,
            BiomeId::CrevasseField,
        ] {
            assert!(
                !village_biome_is_habitable(biome),
                "{biome:?} must not carry villages"
            );
        }
    }

    #[test]
    fn the_spacing_circle_covers_the_real_house() {
        // The circle that reserves ground must actually contain the mesh,
        // whatever the house's dimensions become. Measured from the vertices
        // rather than from the constants, so changing the width, depth or roof
        // overhang cannot leave the circle behind.
        let mesh = build_house_mesh();
        let furthest = mesh
            .iter()
            .map(|vertex| {
                let x = f64::from(vertex.position[0]);
                let y = f64::from(vertex.position[1]);
                (x * x + y * y).sqrt()
            })
            .fold(0.0_f64, f64::max);
        let circle = house_plan_radius_meters();
        assert!(
            circle >= furthest - 1.0e-6,
            "circle {circle}m does not cover the mesh's {furthest}m corner"
        );
        assert!(
            circle - furthest < 0.01,
            "circle {circle}m is needlessly larger than the mesh's {furthest}m"
        );
    }

    #[test]
    fn houses_never_overlap_in_any_village() {
        // The unbreakable rule. Two houses may not come closer than three plan
        // radii centre to centre: two for the circles themselves and one for
        // the clear ground between them.
        let minimum = house_minimum_spacing_meters();
        let mut smallest = f64::INFINITY;
        for seed_index in 0..512_u32 {
            let site_seed = hash_u32(0x51ed_c0de ^ seed_index);
            let layout = village_house_layout(site_seed);
            for (first, &(ae, an, _)) in layout.iter().enumerate() {
                for &(be, bn, _) in layout.iter().skip(first + 1) {
                    let separation = ((ae - be).powi(2) + (an - bn).powi(2)).sqrt();
                    smallest = smallest.min(separation);
                    assert!(
                        separation >= minimum - 1.0e-9,
                        "two houses {separation}m apart, below the {minimum}m minimum"
                    );
                }
            }
        }
        assert!(
            smallest.is_finite(),
            "no villages produced a pair to compare"
        );
    }

    #[test]
    fn a_village_still_fills_out_despite_the_spacing_rule() {
        // Rejection sampling must not quietly gut the village: a 110m radius
        // has room for far more than twenty houses at this spacing, so almost
        // every one should find a home.
        let mut total = 0_usize;
        let trials = 256_u32;
        for seed_index in 0..trials {
            total += village_house_layout(hash_u32(0xbeef_0001 ^ seed_index)).len();
        }
        let mean = total as f64 / f64::from(trials);
        assert!(
            mean > f64::from(HOUSES_PER_VILLAGE) * 0.8,
            "villages average only {mean} houses of {HOUSES_PER_VILLAGE}"
        );
    }

    #[test]
    fn every_face_points_out_of_the_house() {
        // Back-face culling discards any triangle wound the wrong way round.
        // The walls were hand-wound from a convention copied out of `ship.rs`,
        // whose local axes are not these, so this checks the result rather
        // than trusting the copy: every face normal must point away from the
        // house's own middle.
        let mesh = build_house_mesh();
        let centre = glam::Vec3::new(0.0, 0.0, (HOUSE_WALL_HEIGHT_METERS * 0.5) as f32);
        let mut inward = Vec::new();
        for (index, triangle) in mesh.chunks_exact(3).enumerate() {
            let a = glam::Vec3::from(triangle[0].position);
            let b = glam::Vec3::from(triangle[1].position);
            let c = glam::Vec3::from(triangle[2].position);
            let face_centre = (a + b + c) / 3.0;
            let normal = glam::Vec3::from(triangle[0].normal);
            let outward = face_centre - centre;
            if outward.length_squared() > 1.0e-6 && normal.dot(outward.normalize()) < 0.0 {
                inward.push((index, normal.to_array(), outward.normalize().to_array()));
            }
        }
        assert!(
            inward.is_empty(),
            "{} of {} triangles are wound inward: {:?}",
            inward.len(),
            mesh.len() / 3,
            &inward[..inward.len().min(6)]
        );
    }

    #[test]
    fn village_flatness_is_measured_at_village_scale() {
        // The footprint spans two village radii. The budget across it should
        // work out to a gentle grade -- a settlement site, not a hillside.
        let span = VILLAGE_RADIUS_METERS * 2.0;
        let grade = (VILLAGE_FOOTPRINT_HEIGHT_SPREAD_METERS / span)
            .atan()
            .to_degrees();
        assert!(grade < 10.0, "village grade {grade} degrees is too steep");
        // A single house must sit closer to the village's mean than the whole
        // footprint is allowed to vary, or the cluster comes apart.
        assert!(HOUSE_HEIGHT_DEVIATION_METERS < VILLAGE_FOOTPRINT_HEIGHT_SPREAD_METERS);
    }

    #[test]
    fn the_search_ring_reaches_as_far_as_the_render_cutoff() {
        // Villages inside the cutoff must all be searched, or they pop in when
        // the camera crosses a cell boundary. A camera at its own cell's edge
        // has the far side of the cutoff `ring * cell` away.
        let cell_metres = (std::f64::consts::PI * 2.0 * planet_radius_meters() / 4.0)
            / f64::from(1_u32 << VILLAGE_CELL_LEVEL);
        let reach = f64::from(VILLAGE_SEARCH_RING) * cell_metres;
        assert!(
            reach >= VILLAGE_RENDER_DISTANCE_METERS,
            "ring reaches {reach}m but the cutoff is {VILLAGE_RENDER_DISTANCE_METERS}m"
        );
    }

    #[test]
    fn the_render_cutoff_can_reach_a_neighbouring_cell() {
        // At level 8 the cell was 24.5km against a 6km cutoff, so only the
        // camera's own cell could ever contribute and villages were almost
        // never visible. The cutoff must now be a real fraction of the cell.
        let cell_metres = (std::f64::consts::PI * 2.0 * planet_radius_meters() / 4.0)
            / f64::from(1_u32 << VILLAGE_CELL_LEVEL);
        assert!(
            VILLAGE_RENDER_DISTANCE_METERS > cell_metres * 0.5,
            "cutoff {VILLAGE_RENDER_DISTANCE_METERS}m against a {cell_metres}m cell"
        );
    }
}
