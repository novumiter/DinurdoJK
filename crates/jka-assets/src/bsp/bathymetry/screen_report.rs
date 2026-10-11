//! Offline candidate diagnostics, separate from allocation and frame work.
use super::*;

impl Geometry {
    /// Offline diagnostics only: complete candidate tile counts and a few
    /// resolved column samples explain conservative-screen false positives.
    /// Samples are not an allocation proof or a shoreline eligibility test.
    pub fn shoreline_screening_report(
        &self,
        regions: &[Region],
        max_depth: f32,
    ) -> Result<String, String> {
        if !max_depth.is_finite() || max_depth <= 0.0 || regions.is_empty() {
            return Err("invalid screening report inputs".into());
        }
        let first = &regions[0];
        if regions.iter().any(|r| {
            r.slot != first.slot
                || !r.level.is_finite()
                || r.level != first.level
                || r.bounds.is_none_or(|b| {
                    b.iter().flatten().any(|v| !v.is_finite()) || (0..2).any(|a| b[0][a] >= b[1][a])
                })
        }) {
            return Err("screening report requires one tiled pool".into());
        }
        let pool_bounds = bounds(regions.iter().flat_map(|r| r.bounds.unwrap()));
        let mut union = HashSet::new();
        let mut rows = Vec::new();
        for c in self.shoreline_candidates(first.level, max_depth, pool_bounds) {
            let b = bounds(c.polygon.iter().map(|p| p.map(|v| v as f32)));
            let mut touched = 0;
            for (id, r) in regions.iter().enumerate() {
                let rectangle = r.bounds.unwrap();
                if (0..2).all(|a| {
                    b[1][a] + EPS as f32 >= rectangle[0][a]
                        && b[0][a] - EPS as f32 <= rectangle[1][a]
                }) && overlaps_polygon(&c.polygon, rectangle)
                {
                    touched += 1;
                    union.insert(id);
                }
            }
            if touched == 0 {
                continue;
            }
            rows.push((touched, c));
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.source.cmp(&b.1.source)));
        let count = rows.len();
        let mut output = Vec::new();
        for (touched, c) in rows.into_iter().take(32) {
            let b = bounds(c.polygon.iter().map(|p| p.map(|v| v as f32)));
            let center: [f64; 2] = std::array::from_fn(|a| {
                c.polygon.iter().map(|p| p[a]).sum::<f64>() / c.polygon.len() as f64
            });
            let mut samples = Vec::new();
            for p in std::iter::once(center).chain(
                c.polygon
                    .iter()
                    .map(|p| std::array::from_fn(|a| center[a] * 0.01 + p[a] * 0.99)),
            ) {
                let p = p.map(|v| v as f32);
                let solids: Vec<_> = self
                    .solids
                    .iter()
                    .enumerate()
                    .filter_map(|(i, s)| {
                        (0..2)
                            .all(|a| p[a] >= s.bounds[0][a] && p[a] <= s.bounds[1][a])
                            .then_some(i)
                    })
                    .collect();
                let triangles: Vec<_> = self
                    .triangles
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| {
                        (0..2)
                            .all(|a| p[a] >= t.bounds[0][a] && p[a] <= t.bounds[1][a])
                            .then_some(i)
                    })
                    .collect();
                let col = self.column(p, first.level, &solids, &triangles);
                samples.push(format!(
                    "({:.1},{:.1}): bed={:?}, overhang={}, footprint={}",
                    p[0],
                    p[1],
                    col.bed,
                    col.overhang,
                    regions
                        .iter()
                        .any(|r| r.triangles.iter().any(|&t| contains(t, p)))
                ));
            }
            output.push(format!(
                "{}: {touched} tiles; height={:?}; bounds={b:?}; samples [{}]",
                c.source,
                c.height,
                samples.join("; ")
            ));
        }
        Ok(format!("slot {} level {}u: {} candidate surfaces; {} candidate tiles / {} footprint tiles (before halo)\n{}\n", first.slot, first.level, count, union.len(), regions.len(), output.join("\n")))
    }
}
