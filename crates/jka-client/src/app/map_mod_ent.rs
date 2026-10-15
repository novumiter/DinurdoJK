//! OpenJK-compatible full-entity-lump (.ent) import/export for local placement edits.
//! This editor deliberately imports only *additive* MD3/inline placements;
//! external .ent edits to baked BSP entities require a full entity editor.
use super::*;
use super::egui_model_placement::{BmodelPlacement, ModelPlacement, PlacementKind, bmodel_pivot};
use jka_assets::bsp::{Bsp, Entity, MAX_FILE_BYTES};
use jka_assets::pk3::AssetSearchPath;
use std::path::PathBuf;

const MAX_ENT_BYTES: usize = 16 * 1024 * 1024;

fn property<'a>(entity: &'a Entity, key: &str) -> Option<&'a str> {
    entity.properties.iter().rev()
        .find(|(k,_)| k.eq_ignore_ascii_case(key.as_bytes()))
        .and_then(|(_,v)| std::str::from_utf8(v).ok())
}

fn entity(pairs: &[(&str, String)]) -> Entity {
    Entity { properties: pairs.iter().map(|(k,v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec())).collect() }
}

fn xyz(v: [f32; 3]) -> String {
    format!("{:.6} {:.6} {:.6}", v[0], v[1], v[2])
}
fn parse_xyz(text: &str) -> Result<[f32; 3], String> {
    let components: Vec<_> = text.split_ascii_whitespace().collect();
    if components.len() != 3 { return Err(format!("expected 3 coordinates, got {text:?}")); }
    let mut out = [0.0; 3];
    for (i, part) in components.iter().enumerate() {
        out[i] = part.parse::<f32>().map_err(|_| format!("invalid coordinate {part:?}"))?;
        if !out[i].is_finite() { return Err("non-finite coordinate".into()); }
    }
    Ok(out)
}
fn angles(entity: &Entity) -> Result<[f32; 3], String> {
    if let Some(value) = property(entity, "angles") { return parse_xyz(value); }
    if let Some(value) = property(entity, "angle") {
        let yaw: f32 = value.parse().map_err(|_| "invalid yaw".to_owned())?;
        return if yaw.is_finite() { Ok([0.0, yaw, 0.0]) } else { Err("non-finite yaw".into()) };
    }
    Ok([0.0; 3])
}
fn point(entity: &Entity) -> Result<[f32;3], String> {
    parse_xyz(property(entity, "origin").ok_or("entity has no origin")?)
}

/// The engine's token grammar: braces, quoted byte strings, ASCII whitespace,
/// and line comments. Do not interpret JSON/C escapes that the engine does not.
fn parse_ent(data: &[u8]) -> Result<Vec<Entity>, String> {
    if data.len() > MAX_ENT_BYTES { return Err(".ent exceeds 16 MiB safety limit".into()); }
    fn space(bytes: &[u8], at: &mut usize) {
        loop {
            while bytes.get(*at).is_some_and(|b| b.is_ascii_whitespace() || *b == 0) { *at += 1; }
            if bytes.get(*at..*at+2) != Some(b"//") { break; }
            while bytes.get(*at).is_some_and(|b| *b != b'\n') { *at += 1; }
        }
    }
    fn token(bytes: &[u8], at: &mut usize) -> Result<Vec<u8>, String> {
        space(bytes, at);
        if bytes.get(*at) != Some(&b'"') { return Err(format!("expected quoted key/value at byte {}", at)); }
        *at += 1;
        let begin = *at;
        while let Some(&c) = bytes.get(*at) {
            if c == b'"' {
                let result = bytes[begin..*at].to_vec();
                *at += 1;
                if result.len() >= 1024 { return Err("entity token exceeds MAX_TOKEN_CHARS".into()); }
                return Ok(result);
            }
            if c == 0 || c == b'\n' || c == b'\r' { return Err("invalid newline/NUL in entity token".into()); }
            *at += 1;
        }
        Err("unterminated quoted token".into())
    }
    let mut at = 0;
    let mut entities = Vec::new();
    loop {
        space(data, &mut at);
        if at == data.len() { break; }
        if data.get(at) != Some(&b'{') { return Err(format!("expected opening brace at byte {at}")); }
        at += 1;
        let mut pairs = Vec::new();
        let mut chars = 0;
        loop {
            space(data, &mut at);
            if data.get(at) == Some(&b'}') { at += 1; break; }
            if at == data.len() { return Err("unclosed entity".into()); }
            let key = token(data, &mut at)?;
            let value = token(data, &mut at)?;
            chars += key.len() + 1 + value.len() + 1;
            if pairs.len() >= 64 || chars > 4096 { return Err("entity exceeds engine spawnvar limits".into()); }
            pairs.push((key, value));
        }
        entities.push(Entity { properties: pairs });
    }
    Ok(entities)
}

fn write_ent(entities: &[Entity]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.extend_from_slice(b"// Full JKA entity-lump replacement (.ent). Includes original BSP entities.\n");
    for entity in entities {
        out.extend_from_slice(b"{\n");
        let mut chars = 0;
        if entity.properties.len() > 64 { return Err("entity exceeds MAX_SPAWN_VARS".into()); }
        for (key, value) in &entity.properties {
            chars += key.len() + 1 + value.len() + 1;
            if key.len() >= 1024 || value.len() >= 1024 || chars > 4096 {
                return Err("entity exceeds engine spawnvar limits".into());
            }
            for data in [key, value] {
                if data.contains(&b'"') || data.contains(&b'\n') || data.contains(&b'\r') || data.contains(&0) {
                    return Err("entity contains unrepresentable quoted token".into());
                }
            }
            out.extend_from_slice(b"\""); out.extend_from_slice(key); out.extend_from_slice(b"\" \"");
            out.extend_from_slice(value); out.extend_from_slice(b"\"\n");
        }
        out.extend_from_slice(b"}\n");
        if out.len() > MAX_ENT_BYTES { return Err("export exceeds 16 MiB".into()); }
    }
    Ok(out)
}

fn bmodel_entity(p: &BmodelPlacement) -> Entity {
    // Bmodel geometry stays in BSP-world coordinates. A mover's origin is the
    // translation offset; the editor's crosshair hit is the desired *pivot*.
    // No modelscale key: the server/CS_MODELS path has no brush scale.
    let instance = p.instance();
    entity(&[("classname", "func_static".into()), ("model", format!("*{}", p.model)),
        ("origin", xyz(instance.origin)), ("angles", xyz(p.angles)),
        ("_dinurdo_placement", "bmodel".into())])
}
fn md3_entity(p: &ModelPlacement) -> Entity {
    entity(&[("classname", "misc_model_static".into()), ("model", p.qpath.clone()),
        ("origin", xyz(p.origin)), ("angles", xyz(p.angles)),
        ("modelscale", format!("{:.6}", p.scale)),
        ("_dinurdo_placement", "md3".into())])
}

fn clean_map_name(label: &str) -> Result<&str, String> {
    let name = label.strip_suffix(".bsp").ok_or(".ent export requires a loaded BSP map")?;
    if name.is_empty() || name.split('/').any(|part| part.is_empty() ||
        !part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')) {
        return Err("invalid BSP map name".into());
    }
    Ok(name)
}
fn clean_ent_name(file: &str) -> Result<&str, String> {
    let file = file.trim();
    let basename = file.strip_suffix(".ent").ok_or("filename must end in .ent")?;
    if basename.is_empty() || basename.split('/').any(|part| part.is_empty() ||
        !part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')) {
        return Err("use a relative name like mp/ffa3.ent (no paths outside mapmods)".into())
    }
    Ok(file)
}

impl App {
    fn map_mod_path(&self) -> Result<PathBuf, String> {
        clean_map_name(&self.model_placement.map_label)?;
        let name = clean_ent_name(&self.model_placement.ent_filename)?;
        Ok(self.game.as_deref().unwrap_or(&self.base).join("mapmods").join(name))
    }
    fn map_mod_base_entities(&self) -> Result<Vec<Entity>, String> {
        let name = clean_map_name(&self.model_placement.map_label)?;
        let mut vfs = AssetSearchPath::open_game(&self.base, self.game.as_deref())
            .map_err(|error| error.to_string())?;
        if let Some(cache) = &self.prepared_map_cache {
            vfs.set_allow_asset_overrides(cache.prepare_options.allow_asset_overrides);
        }
        let path = format!("maps/{name}.bsp");
        let asset = vfs.read(&path, MAX_FILE_BYTES).map_err(|error| error.to_string())?
            .ok_or_else(|| format!("map asset {path} not found"))?;
        Ok(Bsp::parse(&asset.bytes).map_err(|error| error.to_string())?.entities)
    }
    pub(in crate::app) fn open_map_mods(&mut self) {
        let Some(cache) = &self.prepared_map_cache else {
            self.console_status = "MAP MODS: LOAD A BSP FIRST".into();
            return;
        };
        let Ok(map_name) = clean_map_name(&cache.label) else {
            self.console_status = "MAP MODS: ONLY BSP MAPS ARE SUPPORTED".into();
            return;
        };
        self.model_placement.map_label = cache.label.clone();
        if self.model_placement.ent_filename.is_empty() {
            self.model_placement.ent_filename = format!("{map_name}.ent");
        }
        self.set_overlay(OverlayMode::MapMods);
    }
    fn export_map_mod(&mut self) -> Result<String, String> {
        let base = self.map_mod_base_entities()?;
        let mut merged = base;
        merged.extend(self.model_placement.placed.iter().map(md3_entity));
        merged.extend(self.model_placement.bmodel_placed.iter().map(bmodel_entity));
        let data = write_ent(&merged)?;
        let path = self.map_mod_path()?;
        if path.exists() && !self.model_placement.ent_overwrite {
            return Err(format!("File exists: {}. Check 'Allow overwrite' to replace it.", path.display()));
        }
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        // Write to a new adjacent file first; no truncated exports on a failed write.
        let temp = path.with_extension("ent.tmp");
        std::fs::write(&temp, data).map_err(|e| e.to_string())?;
        let backup = path.with_extension("ent.backup");
        let had_old = path.exists();
        if had_old {
            if backup.exists() {
                let _ = std::fs::remove_file(&temp);
                return Err(format!("Backup {} already exists; move it before exporting", backup.display()));
            }
            if let Err(error) = std::fs::rename(&path, &backup) {
                let _ = std::fs::remove_file(&temp);
                return Err(error.to_string());
            }
        }
        if let Err(error) = std::fs::rename(&temp, &path) {
            if had_old { let _ = std::fs::rename(&backup, &path); }
            return Err(format!("Unable to finalize .ent export: {error}"));
        }
        if had_old { let _ = std::fs::remove_file(&backup); }
        Ok(format!("Exported {} MD3 + {} BMODEL placements with BSP entities to {}",
            self.model_placement.placed.len(), self.model_placement.bmodel_placed.len(), path.display()))
    }
    fn import_map_mod(&mut self) -> Result<String, String> {
        let base = self.map_mod_base_entities()?;
        let path = self.map_mod_path()?;
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let all = parse_ent(&bytes)?;
        if all.len() < base.len() || !all.iter().take(base.len())
            .zip(base.iter()).all(|(left,right)| left.properties == right.properties) {
            return Err("The .ent modifies/removes/reorders the original BSP entities. This placement importer supports additive .ent files only; no changes were applied.".into());
        }
        let cache = self.prepared_map_cache.as_ref().ok_or("no prepared map")?;
        let mut md3 = Vec::new();
        let mut bsp_models = Vec::new();
        for (i, entity) in all.iter().enumerate().skip(base.len()) {
            let cls = property(entity, "classname").unwrap_or("");
            if cls.eq_ignore_ascii_case("misc_model_static") {
                let qpath = property(entity, "model").ok_or(format!("entity {i}: model missing"))?;
                if !qpath.to_ascii_lowercase().ends_with(".md3") { return Err(format!("entity {i}: only MD3 placements are supported")); }
                let scale: f32 = property(entity, "modelscale").unwrap_or("1").parse()
                    .map_err(|_| format!("entity {i}: invalid modelscale"))?;
                if !scale.is_finite() || !(0.01..=100.0).contains(&scale) { return Err(format!("entity {i}: invalid scale")); }
                if property(entity, "modelscale_vec").is_some() { return Err(format!("entity {i}: nonuniform modelscale_vec is not supported")); }
                md3.push(ModelPlacement { qpath: qpath.into(), origin: point(entity)?,
                    angles: angles(entity)?, scale, bounds: ([0.0;3], [0.0;3]) });
            } else if cls.eq_ignore_ascii_case("func_static") &&
                property(entity, "model").is_some_and(|s| s.starts_with('*')) {
                let id = property(entity, "model").unwrap()[1..].parse::<u32>()
                    .map_err(|_| format!("entity {i}: invalid inline model handle"))?;
                if id == 0 { return Err(format!("entity {i}: worldspawn *0 is not placeable")); }
                let (pivot, bounds) = bmodel_pivot(&cache.map, id)
                    .ok_or_else(|| format!("entity {i}: *{id} unavailable in this BSP"))?;
                let a = angles(entity)?;
                let offset = point(entity)?;
                let axis = super::egui_model_placement::model_axis(a, 1.0);
                let hit = std::array::from_fn(|j| offset[j] +
                    axis[0][j]*pivot[0] + axis[1][j]*pivot[1] + axis[2][j]*pivot[2]);
                bsp_models.push(BmodelPlacement { model: id, pivot, bounds, hit, angles: a });
            } else {
                return Err(format!("entity {i}: {cls} cannot be represented by the placement editor; no changes applied"));
            }
        }
        // Validate every MD3 and resolve its authored bounds before replacing
        // any placements: failed imports leave current edits untouched.
        if !md3.is_empty() {
            self.ensure_asset_preview_entity_presenter()?;
            let presenter = self.asset_preview_entity_presenter.as_mut().unwrap();
            for placement in &mut md3 {
                placement.bounds = presenter.md3_frame_bounds(&placement.qpath)
                    .map_err(|error| format!("{}: {error}", placement.qpath))?;
            }
        }
        self.model_placement.placed = md3;
        self.model_placement.bmodel_placed = bsp_models;
        self.model_placement.bmodel_collision_dirty = true;
        self.model_placement.placed_dirty = true;
        self.model_placement.active = false;
        self.model_placement.rotating = false;
        self.model_placement.hit_valid = false;
        self.model_placement.preview_key = None;
        self.model_placement.preview_draws = Arc::new(Vec::new());
        self.ensure_inline_instance_capacity();
        Ok(format!("Imported {} MD3 + {} BMODEL placements from {}",
            self.model_placement.placed.len(), self.model_placement.bmodel_placed.len(), path.display()))
    }
    pub(super) fn egui_map_mods(&mut self, root: &mut egui::Ui) {
        let mut action = None;
        let mut close = false;
        egui::Window::new("MAP MODS · .ENT IMPORT / EXPORT")
            .id(egui::Id::new("map_mod_ent_dialog"))
            .default_width(610.0).resizable(true).collapsible(false)
            .show(root.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.heading("MAP MODS");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("CLOSE").clicked() { close = true; }
                    });
                });
                ui.label(format!("BSP: {}", self.model_placement.map_label));
                ui.label(format!("Staged placements: {} MD3 / {} BMODEL",
                    self.model_placement.placed.len(), self.model_placement.bmodel_placed.len()));
                ui.separator();
                ui.label("File name (relative to mapmods/):");
                ui.add(egui::TextEdit::singleline(&mut self.model_placement.ent_filename)
                    .desired_width(480.0).hint_text("mp/ffa3.ent"));
                let root_dir = self.game.as_deref().unwrap_or(&self.base).join("mapmods");
                ui.small(format!("Folder: {}", root_dir.display()));
                ui.horizontal(|ui| {
                    if ui.button("IMPORT .ENT (replace local placements)").clicked() { action = Some(false); }
                    if ui.button("EXPORT .ENT").clicked() { action = Some(true); }
                });
                ui.checkbox(&mut self.model_placement.ent_overwrite, "Allow export to overwrite existing .ent file");
                ui.separator();
                ui.label("Export writes a complete OpenJK-style entity list: original BSP entities plus your placements.");
                ui.label("Import is additive-only: it rejects modified original entities and unknown additions to prevent silent map corruption.");
                ui.label("BMODEL geometry remains unscaled. Server behavior/collision depends on the host game module; imports are local previews only.");
                if !self.model_placement.ent_status.is_empty() { ui.label(&self.model_placement.ent_status); }
            });
        if close { self.set_overlay(OverlayMode::None); }
        if let Some(export) = action {
            let result = if export { self.export_map_mod() } else { self.import_map_mod() };
            self.model_placement.ent_status = match result {
                Ok(msg) => msg,
                Err(err) => format!(".ENT {} FAILED: {err}", if export { "EXPORT" } else { "IMPORT" }),
            };
            self.egui_repaint_requested = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_duplicate_properties_and_original_entity_order() {
        let original = vec![entity(&[("classname", "worldspawn".into()), ("music", "a".into()),
            ("music", "b".into())]), entity(&[("classname", "func_door".into()), ("model", "*1".into())])];
        assert_eq!(parse_ent(&write_ent(&original).unwrap()).unwrap()[0].properties, original[0].properties);
    }
    #[test]
    fn bmodel_placement_exports_unscaled_world_offset() {
        let bmodel = BmodelPlacement { model: 2, pivot: [200.0, 10.0, 30.0],
            hit: [5.0, 15.0, 25.0], angles: [0.0; 3], bounds: ([0.0; 3], [1.0; 3]) };
        let output = bmodel_entity(&bmodel);
        assert_eq!(property(&output, "model"), Some("*2"));
        assert!(property(&output, "modelscale").is_none());
        assert_eq!(point(&output).unwrap(), [-195.0, 5.0, -5.0]);
    }
    #[test]
    fn input_rejects_invalid_or_escaping_filenames() {
        assert!(clean_ent_name("mp/duel1.ent").is_ok());
        assert!(clean_ent_name("../evil.ent").is_err());
        assert!(clean_ent_name("C:/evil.ent").is_err());
    }
    #[test]
    fn rejects_truncated_bad_ent_files() {
        assert!(parse_ent(b"{\n\"classname\" \"worldspawn\"\n").is_err());
        assert!(parse_ent(b"{\"a\" \"b\"} extra").is_err());
    }
}
