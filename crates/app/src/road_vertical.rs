//! Generation-time vertical curves. Geometry, excavation and collision all use
//! the resulting stations; there is no camera-only or render-time smoothing.

/// At most ten percentage points per 20 m (50 m from flat to 25%).
/// Broad filtering below normally produces gentler curves; this ceiling also
/// accommodates short settlement approaches and flat mountain junctions.
pub(crate) const MAX_GRADE_CHANGE_PER_METER: f64 = 0.005;
const TOLERANCE: f64 = 0.00002;
const MAX_SWEEPS: usize = 40000;

struct Constraint {
    terms: Vec<(usize, f64)>,
    lo: f64,
    hi: f64,
}

impl Constraint {
    fn new(terms: &[(usize, f64)], lo: f64, hi: f64) -> Self {
        Self {
            terms: terms.to_vec(),
            lo,
            hi,
        }
    }

    fn value(&self, heights: &[f64]) -> f64 {
        self.terms.iter().map(|&(i, w)| heights[i] * w).sum()
    }

    fn error(&self, heights: &[f64]) -> f64 {
        let value = self.value(heights);
        (self.lo - value).max(value - self.hi).max(0.0)
    }

    fn satisfied(&self, heights: &[f64]) -> bool {
        // Short horizontal-curve stations need proportionally tighter grade
        // tolerances; an absolute height tolerance could exhaust their margin.
        self.error(heights) <= TOLERANCE.min((self.hi - self.lo).abs() * 0.0002)
    }
}

/// Links carry conservative physical horizontal lengths. A zero grade limit
/// on a link preserves an existing flat apron. Caps include pier/roof limits;
/// anchors must remain exact, including the two sides of a region gateway.
pub(crate) fn smooth(
    heights: &mut [f64],
    caps: &[f64],
    anchors: &[(usize, f64)],
    links: &[(usize, usize, f64, f64)],
    cancelled: impl Fn() -> bool,
) -> bool {
    // Collapse flat aprons before solving. Their height is one variable, not
    // an approximate equality that could leave tiny steps at a junction.
    fn root(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = root(parent, parent[i]);
        }
        parent[i]
    }
    let mut parent: Vec<_> = (0..heights.len()).collect();
    for &(a, b, length, grade) in links {
        if grade == 0.0 || length < 0.001 {
            let a = root(&mut parent, a);
            let b = root(&mut parent, b);
            parent[a] = b;
        }
    }
    let groups: Vec<_> = (0..heights.len()).map(|i| root(&mut parent, i)).collect();
    let mut constraints = Vec::new();
    let floor = heights.iter().copied().fold(f64::INFINITY, f64::min);
    let mut bounds: Vec<_> = caps.iter().map(|&cap| (floor, cap)).collect();
    for &(i, height) in anchors {
        let group = groups[i];
        if bounds[group].0 == bounds[group].1 && (bounds[group].0 - height).abs() > TOLERANCE {
            return false;
        }
        bounds[group] = (height, height);
    }
    for (i, &cap) in caps.iter().enumerate() {
        bounds[groups[i]].1 = bounds[groups[i]].1.min(cap);
    }
    if bounds.iter().any(|&(lo, hi)| lo > hi) {
        return false;
    }
    let mut adjacency = vec![Vec::new(); heights.len()];
    for &(a, b, length, grade) in links {
        constraints.push(Constraint::new(
            &[(a, -1.0), (b, 1.0)],
            -grade * length,
            grade * length,
        ));
        if length > 0.001 {
            adjacency[a].push((b, length));
            adjacency[b].push((a, length));
        }
    }
    for (i, adjacent) in adjacency.iter().enumerate() {
        if let &[(a, la), (b, lb)] = adjacent.as_slice() {
            // Grade differences are measured between segment midpoints. Use
            // unequal station lengths too, including short final stations.
            let limit = MAX_GRADE_CHANGE_PER_METER * 0.999 * (la + lb) * 0.5;
            constraints.push(Constraint::new(
                &[(a, 1.0 / la), (i, -1.0 / la - 1.0 / lb), (b, 1.0 / lb)],
                -limit,
                limit,
            ));
        }
    }
    for (i, &(lo, hi)) in bounds.iter().enumerate() {
        if groups[i] == i {
            constraints.push(Constraint::new(&[(i, 1.0)], lo, hi));
        }
    }
    for c in &mut constraints {
        let mut terms: Vec<(usize, f64)> = Vec::new();
        for &(i, weight) in &c.terms {
            let group = groups[i];
            let (lo, hi) = bounds[group];
            if lo == hi {
                c.lo -= weight * lo;
                c.hi -= weight * lo;
            } else if let Some((_, w)) = terms.iter_mut().find(|(j, _)| *j == group) {
                *w += weight;
            } else {
                terms.push((group, weight));
            }
        }
        terms.retain(|&(_, w)| w.abs() > 1e-12);
        c.terms = terms;
    }
    if constraints
        .iter()
        .any(|c| c.terms.is_empty() && (c.lo > TOLERANCE || c.hi < -TOLERANCE))
    {
        return false;
    }
    constraints.retain(|c| !c.terms.is_empty());
    for c in &mut constraints {
        let norm = c.terms.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
        for (_, w) in &mut c.terms {
            *w /= norm;
        }
        c.lo /= norm;
        c.hi /= norm;
    }
    let mut rows = vec![0.0; heights.len()];
    for c in &constraints {
        let sum: f64 = c.terms.iter().map(|(_, w)| w.abs()).sum();
        for &(i, w) in &c.terms {
            rows[i] += w.abs() * sum;
        }
    }
    let step = 1.0 / rows.into_iter().fold(1.0, f64::max);
    // Start with broad curves, not the terrain's little oscillations. The
    // constrained solve below is still the acceptance test: filtering alone
    // followed by cap/anchor clipping can leave sharp gradient transitions.
    let mut neighbours = vec![Vec::new(); heights.len()];
    for &(a, b, length, _) in links {
        let (a, b) = (groups[a], groups[b]);
        if a != b {
            neighbours[a].push((b, 1.0 / length));
            neighbours[b].push((a, 1.0 / length));
        }
    }
    let mut old = heights.to_vec();
    for sweep in 0..512 {
        if sweep % 64 == 0 && cancelled() {
            return false;
        }
        old.copy_from_slice(heights);
        for (i, adjacent) in neighbours.iter().enumerate() {
            if !adjacent.is_empty() {
                let total: f64 = adjacent.iter().map(|(_, w)| w).sum();
                let mean: f64 = adjacent.iter().map(|&(j, w)| old[j] * w).sum::<f64>() / total;
                heights[i] = ((old[i] + mean) * 0.5).clamp(bounds[i].0, bounds[i].1);
            }
        }
    }
    let desired = heights.to_vec();
    let mut dual = vec![0.0; constraints.len()];
    let mut momentum = vec![0.0; constraints.len()];
    let mut t: f64 = 1.0;
    // Accelerated dual projections (FISTA). The absolute row-sum bound on
    // A-transpose*A gives a safe step without a dense matrix or dependency.
    for sweep in 0..MAX_SWEEPS {
        if sweep % 32 == 0 && cancelled() {
            return false;
        }
        heights.copy_from_slice(&desired);
        for (c, &v) in constraints.iter().zip(&momentum) {
            for &(i, w) in &c.terms {
                heights[i] -= w * v;
            }
        }
        if sweep % 32 == 0 {
            for (i, &(lo, hi)) in bounds.iter().enumerate() {
                heights[i] = heights[i].clamp(lo, hi);
            }
            if constraints.iter().all(|c| c.satisfied(heights)) {
                for i in 0..heights.len() {
                    heights[i] = heights[groups[i]];
                }
                return true;
            }
            heights.copy_from_slice(&desired);
            for (c, &v) in constraints.iter().zip(&momentum) {
                for &(i, w) in &c.terms {
                    heights[i] -= w * v;
                }
            }
        }
        let next_t = (1.0 + (1.0 + 4.0 * t * t).sqrt()) * 0.5;
        let beta = (t - 1.0) / next_t;
        for ((c, v), y) in constraints.iter().zip(&mut dual).zip(&mut momentum) {
            let x = *y + step * c.value(heights);
            let next = x - step * (x / step).clamp(c.lo, c.hi);
            *y = next + beta * (next - *v);
            *v = next;
        }
        t = next_t;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_curves_preserve_unequal_flat_aprons_anchors_and_cover() {
        let distances = [0.0, 5.0, 25.0, 45.0, 65.0, 85.0, 95.0, 100.0];
        let mut heights: Vec<_> = distances.iter().map(|d| d * 0.03).collect();
        heights[1] = 0.0;
        heights[6] = 3.0;
        let mut caps = vec![32.0; heights.len()];
        caps[4] = 1.8; // A roof/pier cap must survive smoothing too.
        let links: Vec<_> = (1..heights.len())
            .map(|i| {
                (
                    i - 1,
                    i,
                    distances[i] - distances[i - 1],
                    if i == 1 || i == 7 { 0.0 } else { 0.249 },
                )
            })
            .collect();
        assert!(smooth(
            &mut heights,
            &caps,
            &[(0, 0.0), (7, 3.0)],
            &links,
            || false
        ));
        assert_eq!(&heights[..2], &[0.0, 0.0]);
        assert_eq!(&heights[6..], &[3.0, 3.0]);
        assert!(heights[4] <= caps[4]);
        for i in 1..heights.len() - 1 {
            let la = distances[i] - distances[i - 1];
            let lb = distances[i + 1] - distances[i];
            let change =
                ((heights[i + 1] - heights[i]) / lb - (heights[i] - heights[i - 1]) / la).abs();
            assert!(change / ((la + lb) * 0.5) <= MAX_GRADE_CHANGE_PER_METER);
        }
    }

    #[test]
    fn vertical_curve_generation_can_be_superseded() {
        assert!(!smooth(
            &mut [0.0, 1.0, 0.0],
            &[32.0; 3],
            &[],
            &[(0, 1, 20.0, 0.249), (1, 2, 20.0, 0.249)],
            || true
        ));
    }

    #[test]
    fn vertical_curves_round_a_sharp_crest_without_relaxing_grade_or_caps() {
        let mut heights: Vec<_> = (0..101)
            .map(|i| 100.0 - (i as f64 - 50.0).abs() * 4.8)
            .collect();
        let caps: Vec<_> = heights.iter().map(|h| h + 32.0).collect();
        let anchors = [(0, heights[0]), (100, heights[100])];
        let links: Vec<_> = (1..heights.len())
            .map(|i| (i - 1, i, 20.0, 0.249))
            .collect();
        assert!(smooth(&mut heights, &caps, &anchors, &links, || false));
        assert!(
            heights[50] < 90.0,
            "crest must round over, not just retain the kink"
        );
        for w in heights.windows(3) {
            assert!((w[2] - 2.0 * w[1] + w[0]).abs() / 400.0 <= MAX_GRADE_CHANGE_PER_METER + 1e-7);
        }
        for (i, h) in heights.iter().enumerate() {
            assert!(*h <= caps[i]);
        }
        for (i, h) in anchors {
            assert_eq!(heights[i], h);
        }
    }

    #[test]
    fn vertical_curves_reject_an_impossible_short_flat_to_steep_approach() {
        let mut heights = vec![0.0, 0.0, 4.8];
        assert!(!smooth(
            &mut heights,
            &[32.0; 3],
            &[(0, 0.0), (2, 4.8)],
            &[(0, 1, 20.0, 0.0), (1, 2, 20.0, 0.249)],
            || false
        ));
    }
}
