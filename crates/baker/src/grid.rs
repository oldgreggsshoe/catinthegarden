use std::f64::consts::{FRAC_PI_2, PI, TAU};

use catinthegarden_coretypes::PLANET_RADIUS_METERS;
use glam::DVec3;

pub const NEIGHBOR_OFFSETS: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

#[derive(Clone, Debug)]
pub struct SphericalGrid {
    width: usize,
    height: usize,
    directions: Vec<DVec3>,
}

impl SphericalGrid {
    pub fn new(width: usize, height: usize) -> Self {
        let mut directions = Vec::with_capacity(width * height);
        for y in 0..height {
            let latitude = ((y as f64 + 0.5) / height as f64) * PI - FRAC_PI_2;
            for x in 0..width {
                let longitude = ((x as f64 + 0.5) / width as f64) * TAU - PI;
                directions.push(DVec3::new(
                    latitude.cos() * longitude.cos(),
                    latitude.sin(),
                    latitude.cos() * longitude.sin(),
                ));
            }
        }
        Self {
            width,
            height,
            directions,
        }
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub fn len(&self) -> usize {
        self.directions.len()
    }

    pub fn direction(&self, index: usize) -> DVec3 {
        self.directions[index]
    }

    pub fn latitude(&self, index: usize) -> f64 {
        self.directions[index].y.asin()
    }

    pub fn index(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    pub fn coordinates(&self, index: usize) -> (usize, usize) {
        (index % self.width, index / self.width)
    }

    pub fn offset_index(&self, index: usize, dx: isize, dy: isize) -> Option<usize> {
        let (x, y) = self.coordinates(index);
        let next_y = y.checked_add_signed(dy)?;
        if next_y >= self.height {
            return None;
        }
        let next_x = (x as isize + dx).rem_euclid(self.width as isize) as usize;
        Some(self.index(next_x, next_y))
    }

    pub fn neighbor(&self, index: usize, neighbor: usize) -> Option<usize> {
        let (dx, dy) = NEIGHBOR_OFFSETS[neighbor];
        self.offset_index(index, dx, dy)
    }

    pub fn distance_meters(&self, a: usize, b: usize) -> f64 {
        let cosine = self.direction(a).dot(self.direction(b)).clamp(-1.0, 1.0);
        cosine.acos().max(1.0e-12) * PLANET_RADIUS_METERS
    }

    fn sample_coordinates(&self, direction: DVec3) -> (usize, usize, usize, usize, f64, f64) {
        let direction = direction.normalize();
        let longitude = direction.z.atan2(direction.x);
        let latitude = direction.y.asin();
        let sample_x = (longitude + PI) / TAU * self.width as f64 - 0.5;
        let sample_y = ((latitude + FRAC_PI_2) / PI * self.height as f64 - 0.5)
            .clamp(0.0, self.height.saturating_sub(1) as f64);
        let floor_x = sample_x.floor();
        let x0 = floor_x.rem_euclid(self.width as f64) as usize;
        let x1 = (x0 + 1) % self.width;
        let y0 = sample_y.floor() as usize;
        let y1 = (y0 + 1).min(self.height - 1);
        (x0, x1, y0, y1, sample_x - floor_x, sample_y - y0 as f64)
    }

    pub fn sample_f64(&self, values: &[f64], direction: DVec3) -> f64 {
        let (x0, x1, y0, y1, tx, ty) = self.sample_coordinates(direction);
        let top = values[self.index(x0, y0)] * (1.0 - tx) + values[self.index(x1, y0)] * tx;
        let bottom = values[self.index(x0, y1)] * (1.0 - tx) + values[self.index(x1, y1)] * tx;
        top * (1.0 - ty) + bottom * ty
    }

    pub fn sample_u8_linear(&self, values: &[u8], direction: DVec3) -> u8 {
        let (x0, x1, y0, y1, tx, ty) = self.sample_coordinates(direction);
        let promoted = [
            f64::from(values[self.index(x0, y0)]),
            f64::from(values[self.index(x1, y0)]),
            f64::from(values[self.index(x0, y1)]),
            f64::from(values[self.index(x1, y1)]),
        ];
        let top = promoted[0] * (1.0 - tx) + promoted[1] * tx;
        let bottom = promoted[2] * (1.0 - tx) + promoted[3] * tx;
        (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8
    }

    pub fn sample_u8_nearest(&self, values: &[u8], direction: DVec3) -> u8 {
        let direction = direction.normalize();
        let longitude = direction.z.atan2(direction.x);
        let latitude = direction.y.asin();
        let x = (((longitude + PI) / TAU * self.width as f64).floor() as isize)
            .rem_euclid(self.width as isize) as usize;
        let y = (((latitude + FRAC_PI_2) / PI * self.height as f64).floor() as usize)
            .min(self.height - 1);
        values[self.index(x, y)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longitude_wraps_and_latitude_does_not() {
        let grid = SphericalGrid::new(16, 8);
        let left = grid.index(0, 4);
        assert_eq!(grid.offset_index(left, -1, 0), Some(grid.index(15, 4)));
        assert_eq!(grid.offset_index(grid.index(0, 0), 0, -1), None);
    }

    #[test]
    fn negative_longitude_wraps_for_non_power_of_two_width() {
        let grid = SphericalGrid::new(18, 8);
        assert_eq!(
            grid.offset_index(grid.index(0, 4), -1, 0),
            Some(grid.index(17, 4))
        );
        assert_eq!(
            grid.offset_index(grid.index(1, 4), -3, 0),
            Some(grid.index(16, 4))
        );
    }

    #[test]
    fn sampling_is_continuous_across_longitude_seam() {
        let grid = SphericalGrid::new(32, 16);
        let values: Vec<f64> = (0..grid.len())
            .map(|index| grid.direction(index).x)
            .collect();
        let a = grid.sample_f64(&values, DVec3::new(-1.0, 0.0, 1.0e-9));
        let b = grid.sample_f64(&values, DVec3::new(-1.0, 0.0, -1.0e-9));
        assert!((a - b).abs() < 1.0e-8);
    }
}
