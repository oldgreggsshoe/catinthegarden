//! One-time, bounded route search. The corridor is a directed lattice, not a
//! worldwide navigation graph: every edge advances along the survey axis.
use glam::{DVec2, DVec3};

pub const TARGET_LENGTH: f64 = 30_000.0;
pub const SAMPLE_STEP: f64 = 10.0;
pub const MAX_GRADE: f64 = 0.25;
// Reserve one percentage point for the changing local vertical over 30 km.
const LOCAL_GRADE: f64 = MAX_GRADE - 0.01;
const ROW_STEP: f64 = 100.0;
const COLUMN_STEP: f64 = 50.0;
const COLUMNS: usize = 121;
const HEADINGS: usize = 5;
const ROWS: usize = 301;
const DECK_CLEARANCE: f64 = 8.0;

#[derive(Clone, Debug)]
pub struct Route {
    /// Uniform survey-axis spacing, except possibly the final segment.
    pub points: Vec<DVec3>,
    pub ground: Vec<f64>,
    pub distances: Vec<f64>,
    pub straight_cut_volume: f64,
    pub cut_volume: f64,
}

impl Route {
    pub fn plan(mut ground_at: impl FnMut(DVec2) -> Option<f64>) -> Option<Self> {
        let mut grid = vec![f64::NAN; ROWS * COLUMNS];
        for row in 0..ROWS {
            for column in 0..COLUMNS {
                let p = DVec2::new(column_x(column), row as f64 * ROW_STEP);
                if let Some(h) = ground_at(p).filter(|h| h.is_finite()) {
                    grid[row * COLUMNS + column] = h;
                }
            }
        }
        let mut previous = vec![f64::INFINITY; COLUMNS * HEADINGS];
        previous[(COLUMNS / 2) * HEADINGS + 2] = 0.0;
        let mut parents = vec![u8::MAX; ROWS * COLUMNS * HEADINGS];
        for row in 1..ROWS {
            let mut next = vec![f64::INFINITY; previous.len()];
            for column in 0..COLUMNS {
                let height = grid[row * COLUMNS + column];
                if !height.is_finite() {
                    continue;
                }
                for heading in 0..HEADINGS {
                    let shift = heading as isize - 2;
                    let from = column as isize - shift;
                    if !(0..COLUMNS as isize).contains(&from) {
                        continue;
                    }
                    let from = from as usize;
                    let old_height = grid[(row - 1) * COLUMNS + from];
                    if !old_height.is_finite() {
                        continue;
                    }
                    let distance = ROW_STEP.hypot(shift as f64 * COLUMN_STEP);
                    let rise = (height - old_height).abs();
                    // Length + climbing + a strong proxy for cuts/tall piers
                    // when the natural ground exceeds a comfortable grade.
                    let difficult = (rise - distance * 0.06).max(0.0);
                    let edge = distance + rise * 2.0 + difficult * difficult * 5.0;
                    for old in heading.saturating_sub(1)..=(heading + 1).min(HEADINGS - 1) {
                        let turn = (heading as f64 - old as f64).powi(2) * 30.0;
                        let cost = previous[from * HEADINGS + old] + edge + turn;
                        let index = column * HEADINGS + heading;
                        if cost < next[index] {
                            next[index] = cost;
                            parents[(row * COLUMNS + column) * HEADINGS + heading] = old as u8;
                        }
                    }
                }
            }
            previous = next;
        }
        let mut column = COLUMNS / 2;
        let mut heading = (0..HEADINGS).min_by(|a, b| {
            previous[column * HEADINGS + a].total_cmp(&previous[column * HEADINGS + b])
        })?;
        if !previous[column * HEADINGS + heading].is_finite() {
            return None;
        }
        let mut coarse = vec![0.0; ROWS];
        for row in (1..ROWS).rev() {
            coarse[row] = column_x(column);
            let old = parents[(row * COLUMNS + column) * HEADINGS + heading];
            if old == u8::MAX {
                return None;
            }
            column = (column as isize - (heading as isize - 2)) as usize;
            heading = old as usize;
        }
        // Round lattice corners before the final ground/profile survey. No
        // smoothing is applied after that survey or to an untested live route.
        for _ in 0..6 {
            let old = coarse.clone();
            for i in 1..ROWS - 1 {
                coarse[i] = (old[i - 1] + 2.0 * old[i] + old[i + 1]) * 0.25;
            }
        }
        let mut points: Vec<DVec3> = Vec::new();
        let mut ground = Vec::new();
        let mut distances = Vec::new();
        let mut distance = 0.0;
        for i in 0..=3000 {
            let y = i as f64 * SAMPLE_STEP;
            let at = (y / ROW_STEP).min((ROWS - 1) as f64 - 1.0e-9);
            let row = at.floor() as usize;
            let t = at.fract();
            let a = coarse[row];
            let b = coarse[row + 1];
            let m0 = (b - coarse[row.saturating_sub(1)]) * 0.5;
            let m1 = (coarse[(row + 2).min(ROWS - 1)] - a) * 0.5;
            let x = (2.0 * t * t * t - 3.0 * t * t + 1.0) * a
                + (t * t * t - 2.0 * t * t + t) * m0
                + (-2.0 * t * t * t + 3.0 * t * t) * b
                + (t * t * t - t * t) * m1;
            let p = DVec3::new(x, y, 0.0);
            if let Some(previous) = points.last() {
                distance += (p - *previous).truncate().length();
                if distance > TARGET_LENGTH {
                    break;
                }
            }
            let h = ground_at(p.truncate())?;
            if !h.is_finite() {
                return None;
            }
            points.push(p);
            ground.push(h);
            distances.push(distance);
        }
        let profile = vertical_profile(&ground, &distances);
        for (p, z) in points.iter_mut().zip(profile) {
            p.z = z;
        }
        let cut_volume = estimated_cut_volume(&ground, &points, &distances);
        // A separately surveyed straight alignment, with the same vertical
        // solver, gives an honest earthwork reference rather than a flat deck.
        let straight_ground = (0..=3000)
            .map(|i| ground_at(DVec2::new(0.0, i as f64 * SAMPLE_STEP)))
            .collect::<Option<Vec<_>>>()?;
        let straight_distances = (0..straight_ground.len())
            .map(|i| i as f64 * SAMPLE_STEP)
            .collect::<Vec<_>>();
        let straight_points = vertical_profile(&straight_ground, &straight_distances)
            .into_iter()
            .enumerate()
            .map(|(i, h)| DVec3::new(0.0, i as f64 * SAMPLE_STEP, h))
            .collect::<Vec<_>>();
        let straight_cut_volume =
            estimated_cut_volume(&straight_ground, &straight_points, &straight_distances);
        Some(Self {
            points,
            ground,
            distances,
            straight_cut_volume,
            cut_volume,
        })
    }

    pub fn length(&self) -> f64 {
        *self.distances.last().unwrap()
    }

    pub fn at_distance(&self, distance: f64) -> DVec3 {
        let distance = distance.clamp(0.0, self.length());
        let next = self
            .distances
            .partition_point(|d| *d <= distance)
            .min(self.points.len() - 1)
            .max(1);
        let t = (distance - self.distances[next - 1])
            / (self.distances[next] - self.distances[next - 1]);
        self.points[next - 1].lerp(self.points[next], t)
    }

    pub fn at_y(&self, y: f64) -> DVec3 {
        let at = (y / SAMPLE_STEP).clamp(0.0, (self.points.len() - 1) as f64 - 1.0e-9);
        let i = at.floor() as usize;
        self.points[i].lerp(self.points[i + 1], at.fract())
    }
}

fn column_x(column: usize) -> f64 {
    (column as f64 - (COLUMNS / 2) as f64) * COLUMN_STEP
}

fn vertical_profile(ground: &[f64], distances: &[f64]) -> Vec<f64> {
    let mut heights = ground
        .iter()
        .map(|h| h + DECK_CLEARANCE)
        .collect::<Vec<_>>();
    // Broad vertical curves suppress the little terrain oscillations; the
    // projection enforces the hard grade bound, not a soft cost alone.
    for _ in 0..12 {
        let old = heights.clone();
        for (i, h) in heights.iter_mut().enumerate() {
            let lo = i.saturating_sub(10);
            let hi = (i + 11).min(old.len());
            *h = old[lo..hi].iter().sum::<f64>() / (hi - lo) as f64;
        }
    }
    for _ in 0..8 {
        for i in 1..heights.len() {
            let limit = LOCAL_GRADE * (distances[i] - distances[i - 1]);
            heights[i] = heights[i].clamp(heights[i - 1] - limit, heights[i - 1] + limit);
        }
        for i in (0..heights.len() - 1).rev() {
            let limit = LOCAL_GRADE * (distances[i + 1] - distances[i]);
            heights[i] = heights[i].clamp(heights[i + 1] - limit, heights[i + 1] + limit);
        }
        let old = heights.clone();
        for i in 1..heights.len() - 1 {
            heights[i] = (old[i - 1] + 2.0 * old[i] + old[i + 1]) * 0.25;
        }
    }
    // Bound typical pier height before enforcing grade again. Lower the high
    // side of an infeasible transition rather than raising the valley floor.
    for (h, ground) in heights.iter_mut().zip(ground) {
        *h = h.min(ground + 32.0);
    }
    for i in 1..heights.len() {
        heights[i] =
            heights[i].min(heights[i - 1] + LOCAL_GRADE * (distances[i] - distances[i - 1]));
    }
    for i in (0..heights.len() - 1).rev() {
        heights[i] =
            heights[i].min(heights[i + 1] + LOCAL_GRADE * (distances[i + 1] - distances[i]));
    }
    heights
}

fn estimated_cut_volume(ground: &[f64], points: &[DVec3], distances: &[f64]) -> f64 {
    ground
        .iter()
        .zip(points)
        .enumerate()
        .skip(1)
        .map(|(i, (ground, deck))| {
            let depth = (ground - (deck.z - crate::road::CUT_FLOOR_DEPTH)).max(0.0);
            // 18 m floor, 1:1 side slopes: cross-section area = 18*d + d*d.
            (18.0 * depth + depth * depth) * (distances[i] - distances[i - 1])
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alpine_grade_limit_follows_twenty_percent_ground_without_deep_cuts() {
        assert_eq!(MAX_GRADE, 0.25);
        let route = Route::plan(|p| Some(1800.0 + 0.20 * p.y)).unwrap();
        // The end curves deliberately flatten; test the sustained climb.
        assert!(route.ground.iter().zip(&route.points).skip(100)
            .take(route.ground.len() - 200).all(|(g, p)| g - p.z < 1.0));
        assert!(route.points.windows(2).any(|p| {
            (p[1].z - p[0].z).abs() / (p[1] - p[0]).truncate().length() > 0.19
        }));
    }

    #[test]
    fn route_bends_around_hills_instead_of_cutting_straight_through() {
        let ground = |p: DVec2| {
            Some(
                1800.0
                    + 180.0 * (-(p.x / 350.0).powi(2) - ((p.y - 7000.0) / 350.0).powi(2)).exp()
                    + 160.0
                        * (-((p.x - 200.0) / 400.0).powi(2) - ((p.y - 16000.0) / 300.0).powi(2))
                            .exp(),
            )
        };
        let route = Route::plan(ground).unwrap();
        assert!((TARGET_LENGTH - 15.0..=TARGET_LENGTH).contains(&route.length()));
        assert!(route.points.iter().any(|p| p.x.abs() > 300.0));
        assert!(
            route.cut_volume < route.straight_cut_volume * 0.25,
            "{} vs {}",
            route.cut_volume,
            route.straight_cut_volume
        );
        for (i, p) in route.points.windows(2).enumerate() {
            assert!(
                (p[1].z - p[0].z).abs() / (route.distances[i + 1] - route.distances[i])
                    <= MAX_GRADE
            );
        }
    }
    #[test]
    fn unavailable_corridor_is_rejected_and_flat_ground_stays_straight() {
        assert!(Route::plan(|_| None).is_none());
        let route = Route::plan(|_| Some(1900.0)).unwrap();
        assert!(route.points.iter().all(|p| p.x.abs() < 1.0e-6));
        assert_eq!(route.cut_volume, 0.0);
    }
}
