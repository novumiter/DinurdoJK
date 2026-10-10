//! Versioned, bounded cache decoding. A corrupt or stale file is a cache miss.
use super::*;

pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

impl Atlas {
    pub fn encode(&self, key: [u64; 2]) -> Vec<u8> {
        let mut out = b"JKABATHY".to_vec();
        out.extend(CACHE_VERSION.to_le_bytes());
        for k in key {
            out.extend(k.to_le_bytes());
        }
        out.extend(self.bake_ms.to_le_bytes());
        out.extend((self.grids.len() as u32).to_le_bytes());
        out.extend((self.warnings.len() as u32).to_le_bytes());
        for warning in &self.warnings {
            out.extend((warning.len() as u32).to_le_bytes());
            out.extend(warning.as_bytes());
        }
        for g in &self.grids {
            for v in [g.slot, g.size[0], g.size[1], g.components] {
                out.extend(v.to_le_bytes());
            }
            for v in [g.level, g.origin[0], g.origin[1], g.cell_size] {
                out.extend(v.to_le_bytes());
            }
            out.extend((g.footprint.len() as u32).to_le_bytes());
            for v in g.footprint.iter().flatten().flatten() {
                out.extend(v.to_le_bytes());
            }
            for c in &g.cells {
                out.push(c.class as u8);
                out.push(c.coverage);
                out.extend(c.component.to_le_bytes());
                for v in [c.bed, c.depth, c.ceiling, c.bed_min, c.bed_max] {
                    out.extend(v.to_le_bytes());
                }
            }
            for f in g.faces_x.iter().chain(&g.faces_y) {
                out.push(f.boundary as u8);
                out.extend(f.crest.to_le_bytes());
            }
        }
        out.extend(fingerprint(&out).to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8], key: [u64; 2]) -> Result<Self, String> {
        let fail = || "invalid/stale bathymetry cache".to_owned();
        if bytes.len() < 52 || bytes.len() > 128 * 1024 * 1024 {
            return Err(fail());
        }
        let (data, sum) = bytes.split_at(bytes.len() - 8);
        if fingerprint(data) != u64::from_le_bytes(sum.try_into().map_err(|_| fail())?) {
            return Err(fail());
        }
        let mut reader = Reader {
            bytes: data,
            pos: 0,
        };
        if reader.take(8)? != b"JKABATHY"
            || reader.u32()? != CACHE_VERSION
            || [reader.u64()?, reader.u64()?] != key
        {
            return Err(fail());
        }
        let bake_ms = f64::from_le_bytes(reader.take(8)?.try_into().unwrap());
        if !bake_ms.is_finite() || bake_ms < 0.0 {
            return Err(fail());
        }
        let count = reader.u32()? as usize;
        let warnings = reader.u32()? as usize;
        if count > 64 || warnings > 64 {
            return Err(fail());
        }
        let mut atlas = Atlas {
            grids: Vec::new(),
            warnings: Vec::new(),
            bake_ms,
        };
        for _ in 0..warnings {
            let n = reader.u32()? as usize;
            if n > 16_384 {
                return Err(fail());
            }
            atlas.warnings.push(
                std::str::from_utf8(reader.take(n)?)
                    .map_err(|_| fail())?
                    .to_owned(),
            );
        }
        let mut total = 0usize;
        for _ in 0..count {
            let slot = reader.u32()?;
            let size = [reader.u32()?, reader.u32()?];
            let components = reader.u32()?;
            let level = reader.f32()?;
            let origin = [reader.f32()?, reader.f32()?];
            let cell_size = reader.f32()?;
            let nx = size[0] as usize;
            let ny = size[1] as usize;
            if nx == 0
                || ny == 0
                || nx > 1_048_576
                || ny > 1_048_576
                || !level.is_finite()
                || origin.iter().any(|v| !v.is_finite())
                || !cell_size.is_finite()
                || cell_size < 1.0
            {
                return Err(fail());
            }
            let n = nx.checked_mul(ny).ok_or_else(fail)?;
            total = total.checked_add(n).ok_or_else(fail)?;
            if n > 1_048_576 || total > 2_097_152 || components > n as u32 {
                return Err(fail());
            }
            let triangle_count = reader.u32()? as usize;
            if triangle_count > 1_048_576 {
                return Err(fail());
            }
            let mut footprint = Vec::with_capacity(triangle_count);
            for _ in 0..triangle_count {
                let t = [
                    [reader.f32()?, reader.f32()?],
                    [reader.f32()?, reader.f32()?],
                    [reader.f32()?, reader.f32()?],
                ];
                if t.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(fail());
                }
                footprint.push(t);
            }
            let mut cells = Vec::with_capacity(n);
            for _ in 0..n {
                let class = *Class::ALL.get(reader.u8()? as usize).ok_or_else(fail)?;
                let coverage = reader.u8()?;
                let component = reader.u32()?;
                let bed = reader.f32()?;
                let depth = reader.f32()?;
                let ceiling = reader.f32()?;
                let bed_min = reader.f32()?;
                let bed_max = reader.f32()?;
                if coverage > 5
                    || component > components
                    || [bed, depth, ceiling, bed_min, bed_max]
                        .iter()
                        .any(|v| v.is_infinite())
                    || (class == Class::Wet
                        && (!bed.is_finite()
                            || !depth.is_finite()
                            || depth <= 0.0
                            || component == 0))
                {
                    return Err(fail());
                }
                cells.push(Cell {
                    class,
                    coverage,
                    component,
                    bed,
                    depth,
                    ceiling,
                    bed_min,
                    bed_max,
                });
            }
            let mut read_faces = |n: usize| -> Result<Vec<Face>, String> {
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    let boundary = match reader.u8()? {
                        0 => Boundary::Open,
                        1 => Boundary::Wall,
                        2 => Boundary::Outline,
                        3 => Boundary::Unsupported,
                        4 => Boundary::Bank,
                        _ => return Err(fail()),
                    };
                    let crest = reader.f32()?;
                    if crest.is_infinite() {
                        return Err(fail());
                    }
                    out.push(Face { boundary, crest });
                }
                Ok(out)
            };
            let faces_x = read_faces((nx + 1) * ny)?;
            let faces_y = read_faces(nx * (ny + 1))?;
            atlas.grids.push(Grid {
                slot,
                level,
                origin,
                size,
                cell_size,
                components,
                cells,
                faces_x,
                faces_y,
                footprint,
            });
        }
        if reader.pos != data.len() {
            return Err(fail());
        }
        Ok(atlas)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or("truncated bathymetry cache")?;
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}
