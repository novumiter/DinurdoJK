use super::*;
use std::{fmt::Write as _, path::Path};

impl Atlas {
    pub fn summary(&self) -> String {
        let mut s=format!("BSP coastal geometry\nBake: {:.3} ms; retained grid memory: {} bytes\nCoordinates: JKA XY, Z up; units: JKA world units (40 units/metre in the ocean).\n",self.bake_ms,self.memory_bytes());
        for g in &self.grids {
            let _ = write!(
                s,
                "\nSlot {}: level {}; origin {:?}; {}x{}; spacing {}; components {}\n",
                g.slot, g.level, g.origin, g.size[0], g.size[1], g.cell_size, g.components
            );
            for class in Class::ALL {
                let _ = writeln!(
                    s,
                    "  {}: {}",
                    class.name(),
                    g.cells.iter().filter(|c| c.class == class).count()
                );
            }
            let wet: Vec<_> = g.cells.iter().filter(|c| c.class == Class::Wet).collect();
            if !wet.is_empty() {
                let _ = writeln!(
                    s,
                    "  supported wet depth: {:.3}..{:.3}",
                    wet.iter().map(|c| c.depth).fold(f32::INFINITY, f32::min),
                    wet.iter().map(|c| c.depth).fold(0.0, f32::max)
                );
            }
            for boundary in [
                Boundary::Open,
                Boundary::Bank,
                Boundary::Wall,
                Boundary::Outline,
                Boundary::Unsupported,
            ] {
                let _ = writeln!(
                    s,
                    "  {boundary:?} faces: {}",
                    g.faces_x
                        .iter()
                        .chain(&g.faces_y)
                        .filter(|f| f.boundary == boundary)
                        .count()
                );
            }
        }
        for w in &self.warnings {
            let _ = writeln!(s, "WARNING: {w}");
        }
        s
    }

    /// One self-contained interactive report, suitable for opening without a
    /// game/GPU. Shows sampled terrain, exact footprint boundaries, sealed
    /// wall faces, unsupported cells, depths and wet connectivity independently.
    pub fn write_debug(&self, directory: &Path) -> Result<(), String> {
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        std::fs::write(directory.join("summary.txt"), self.summary()).map_err(|e| e.to_string())?;
        let mut data = String::from("[");
        for (i, g) in self.grids.iter().enumerate() {
            if i > 0 {
                data.push(',');
            }
            let _=write!(data,"{{\"slot\":{},\"level\":{},\"origin\":[{},{}],\"size\":[{},{}],\"dx\":{},\"cells\":[",g.slot,g.level,g.origin[0],g.origin[1],g.size[0],g.size[1],g.cell_size);
            for (k, c) in g.cells.iter().enumerate() {
                if k > 0 {
                    data.push(',');
                }
                let _ = write!(data, "[{},", c.class as u8);
                for v in [c.bed, c.depth, c.ceiling, c.bed_min, c.bed_max] {
                    if v.is_finite() {
                        let _ = write!(data, "{v},");
                    } else {
                        data.push_str("null,");
                    }
                }
                let _ = write!(data, "{},{}]", c.coverage, c.component);
            }
            data.push_str("],\"fx\":[");
            for (k, f) in g.faces_x.iter().enumerate() {
                if k > 0 {
                    data.push(',');
                }
                let _ = write!(data, "{}", f.boundary as u8);
            }
            data.push_str("],\"fy\":[");
            for (k, f) in g.faces_y.iter().enumerate() {
                if k > 0 {
                    data.push(',');
                }
                let _ = write!(data, "{}", f.boundary as u8);
            }
            data.push_str("],\"footprint\":[");
            for (k, t) in g.footprint.iter().enumerate() {
                if k > 0 {
                    data.push(',');
                }
                let _ = write!(
                    data,
                    "[[{},{}],[{},{}],[{},{}]]",
                    t[0][0], t[0][1], t[1][0], t[1][1], t[2][0], t[2][1]
                );
            }
            data.push_str("]}");
        }
        data.push(']');
        let html = include_str!("report.html").replace("__ATLAS__", &data);
        std::fs::write(directory.join("bathymetry.html"), html).map_err(|e| e.to_string())
    }
}
