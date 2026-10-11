//! Immutable CPU spatial queries for the shared water footprint.
//! Built once at upload; queries do not allocate or scan the entire pool.
use super::{MaskTriangle, WaterFootprint};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    lo: [f32; 2],
    hi: [f32; 2],
}
impl Bounds {
    fn intersects(self, other: Self) -> bool {
        (0..2).all(|a| self.hi[a] >= other.lo[a] && self.lo[a] <= other.hi[a])
    }
    fn distance_sq(self, p: [f32; 2]) -> f32 {
        (0..2)
            .map(|a| (p[a] - p[a].clamp(self.lo[a], self.hi[a])).powi(2))
            .sum()
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Node {
    bounds: Bounds,
    start: usize,
    len: usize,
    children: [usize; 2],
}
#[derive(Clone, Debug, Default, PartialEq)]
struct Tree {
    bounds: Vec<Bounds>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}
impl Tree {
    fn new(bounds: Vec<Bounds>) -> Self {
        let mut tree = Self {
            order: (0..bounds.len()).collect(),
            bounds,
            nodes: Vec::new(),
        };
        if !tree.order.is_empty() {
            tree.build(0, tree.order.len());
        }
        tree
    }
    fn build(&mut self, start: usize, len: usize) -> usize {
        let mut bounds = self.bounds[self.order[start]];
        for &i in &self.order[start + 1..start + len] {
            for a in 0..2 {
                bounds.lo[a] = bounds.lo[a].min(self.bounds[i].lo[a]);
                bounds.hi[a] = bounds.hi[a].max(self.bounds[i].hi[a]);
            }
        }
        let node = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            start,
            len,
            children: [0; 2],
        });
        if len > 8 {
            let axis = usize::from(bounds.hi[1] - bounds.lo[1] > bounds.hi[0] - bounds.lo[0]);
            let half = len / 2;
            let boxes = &self.bounds;
            self.order[start..start + len].select_nth_unstable_by(half, |&a, &b| {
                (boxes[a].lo[axis] + boxes[a].hi[axis])
                    .total_cmp(&(boxes[b].lo[axis] + boxes[b].hi[axis]))
            });
            let left = self.build(start, half);
            let right = self.build(start + half, len - half);
            self.nodes[node].children = [left, right];
            self.nodes[node].len = 0;
        }
        node
    }
    fn intersects(&self, bounds: Bounds) -> bool {
        !self.nodes.is_empty() && self.intersects_node(0, bounds)
    }
    fn intersects_node(&self, node: usize, bounds: Bounds) -> bool {
        let n = &self.nodes[node];
        if !n.bounds.intersects(bounds) {
            return false;
        }
        if n.len > 0 {
            return self.order[n.start..n.start + n.len]
                .iter()
                .any(|&i| self.bounds[i].intersects(bounds));
        }
        self.intersects_node(n.children[0], bounds) || self.intersects_node(n.children[1], bounds)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FootprintIndex {
    triangles: Vec<MaskTriangle>,
    triangle_tree: Tree,
    tile_tree: Tree,
}
impl FootprintIndex {
    pub fn new(footprint: &WaterFootprint) -> Self {
        let triangles = footprint.triangles.clone();
        let triangle_tree = Tree::new(
            triangles
                .iter()
                .map(|t| Bounds {
                    lo: [0, 1].map(|a| t.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min)),
                    hi: [0, 1].map(|a| t.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max)),
                })
                .collect(),
        );
        let tile_tree = Tree::new(
            footprint
                .tiles
                .iter()
                .map(|t| Bounds {
                    lo: t.minimum,
                    hi: t.maximum,
                })
                .collect(),
        );
        Self {
            triangles,
            triangle_tree,
            tile_tree,
        }
    }
    pub fn rect_may_contain(&self, lo: [f32; 2], hi: [f32; 2]) -> bool {
        self.tile_tree.intersects(Bounds { lo, hi })
    }
    pub fn closest_point(&self, p: [f32; 2]) -> Option<[f32; 2]> {
        if self.triangle_tree.nodes.is_empty() {
            return None;
        }
        let mut best = ([0.0; 2], f32::INFINITY);
        self.nearest_node(0, p, &mut best);
        best.1.is_finite().then_some(best.0)
    }
    fn nearest_node(&self, node: usize, p: [f32; 2], best: &mut ([f32; 2], f32)) {
        let n = &self.triangle_tree.nodes[node];
        if n.bounds.distance_sq(p) > best.1 {
            return;
        }
        if n.len > 0 {
            for &i in &self.triangle_tree.order[n.start..n.start + n.len] {
                if self.triangle_tree.bounds[i].distance_sq(p) > best.1 {
                    continue;
                }
                let q = closest_triangle(self.triangles[i], p);
                let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                if d < best.1 {
                    *best = (q, d);
                }
                if d == 0.0 {
                    return;
                }
            }
        } else {
            let mut children = n.children;
            if self.triangle_tree.nodes[children[1]].bounds.distance_sq(p)
                < self.triangle_tree.nodes[children[0]].bounds.distance_sq(p)
            {
                children.swap(0, 1);
            }
            self.nearest_node(children[0], p, best);
            if best.1 > 0.0 {
                self.nearest_node(children[1], p, best);
            }
        }
    }
}
fn closest_triangle(t: MaskTriangle, p: [f32; 2]) -> [f32; 2] {
    let orient = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
        (f64::from(b[0]) - f64::from(a[0])) * (f64::from(c[1]) - f64::from(a[1]))
            - (f64::from(b[1]) - f64::from(a[1])) * (f64::from(c[0]) - f64::from(a[0]))
    };
    let area = orient(t[0], t[1], t[2]);
    let signs = [
        orient(t[0], t[1], p),
        orient(t[1], t[2], p),
        orient(t[2], t[0], p),
    ];
    if area.abs() > 1e-12 && (signs.iter().all(|&s| s >= 0.0) || signs.iter().all(|&s| s <= 0.0)) {
        return p;
    }
    let mut best = (t[0], f32::INFINITY);
    for edge in 0..3 {
        let a = t[edge];
        let b = t[(edge + 1) % 3];
        let ab = [b[0] - a[0], b[1] - a[1]];
        let length = ab[0] * ab[0] + ab[1] * ab[1];
        let u = if length > 1e-8 {
            ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length
        } else {
            0.0
        };
        let u = u.clamp(0.0, 1.0);
        let q = [a[0] + u * ab[0], a[1] + u * ab[1]];
        let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
        if d < best.1 {
            best = (q, d);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spatial_queries_match_brute_force_on_sparse_irregular_water() {
        let mut triangles = Vec::new();
        for y in 0..16 {
            for x in 0..16 {
                if x > 3 && y > 3 {
                    continue;
                }
                let lo = [x as f32 * 256.0, y as f32 * 256.0];
                let hi = [lo[0] + 256.0, lo[1] + 256.0];
                triangles.extend_from_slice(&[[lo, [hi[0], lo[1]], hi], [lo, hi, [lo[0], hi[1]]]]);
            }
        }
        let face = super::super::WaterFace {
            plane: 0.0,
            minimum: [0.0; 2],
            maximum: [4096.0; 2],
            material: 0,
            authored_ocean: None,
        };
        let footprint =
            super::super::build_water_footprints(&[face], &[triangles], 256.0).remove(0);
        let index = FootprintIndex::new(&footprint);
        let mut random = 0x12345678u32;
        for _ in 0..1500 {
            let mut next = || {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                (random >> 8) as f32 / 16777216.0 * 8192.0 - 2048.0
            };
            let p = [next(), next()];
            let q = index.closest_point(p).unwrap();
            let best = footprint
                .triangles
                .iter()
                .map(|&t| closest_triangle(t, p))
                .map(|q| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2))
                .fold(f32::INFINITY, f32::min);
            let actual = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
            assert!(
                (actual - best).abs() <= best.max(1.0) * 1e-5,
                "p={p:?} nearest={q:?}"
            );
            let hi = [p[0] + 32.0, p[1] + 32.0];
            let expected = footprint.tiles.iter().any(|t| {
                Bounds {
                    lo: t.minimum,
                    hi: t.maximum,
                }
                .intersects(Bounds { lo: p, hi })
            });
            assert_eq!(index.rect_may_contain(p, hi), expected);
        }
        assert!(
            !index.rect_may_contain([2048.0; 2], [2304.0; 2]),
            "dry L corner stays dry"
        );
    }
}
