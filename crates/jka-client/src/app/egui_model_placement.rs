//! Local, map-scoped MD3 placement from the developer wheel.
//! Placements use the same CGame MD3 material path as map props; no protocol
//! entity is created and nothing is submitted to the game server.
use super::*;
use super::egui_theme as theme;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModelPlacement {
    pub qpath: String,
    /// Native JKA (Z-up) coordinates.
    pub origin: [f32; 3],
    /// Native JKA pitch/yaw/roll, in degrees.
    pub angles: [f32; 3],
    pub scale: f32,
    pub bounds: ([f32; 3], [f32; 3]),
}

/// This BSP model is already authored in world coordinates, often hundreds of
/// units away from (0,0,0). `pivot` captures its original map-space origin.
/// Repositioning must subtract the *transformed* pivot before adding the hit.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BmodelPlacement {
    pub model: u32,
    pub pivot: [f32; 3],
    pub hit: [f32; 3],
    pub angles: [f32; 3],
    pub bounds: ([f32; 3], [f32; 3]),
}

impl BmodelPlacement {
    pub fn instance(&self) -> crate::renderer::InlineModelInstance {
        let axis = model_axis(self.angles, 1.0);
        let rotated_pivot: [f32; 3] = std::array::from_fn(|j| {
            axis[0][j] * self.pivot[0]
                + axis[1][j] * self.pivot[1]
                + axis[2][j] * self.pivot[2]
        });
        crate::renderer::InlineModelInstance {
            model: self.model,
            origin: std::array::from_fn(|j| self.hit[j] - rotated_pivot[j]),
            axis,
            is_portal_entity: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlacementKind { Md3, Bmodel }

pub(super) struct ModelPlacementUi {
    pub kind: PlacementKind,
    pub bmodel_search: String,
    pub bmodel_sort: usize,
    pub bmodel_selection: Option<u32>,
    pub bmodel_pivot: [f32; 3],
    pub bmodel_bounds: Option<([f32; 3], [f32; 3])>,
    pub bmodel_placed: Vec<BmodelPlacement>,
    /// Invalidate only on edits/imports or a transition into/out of local play.
    pub bmodel_collision_dirty: bool,
    pub bmodel_collision_local: bool,
    pub ent_filename: String,
    pub ent_overwrite: bool,
    pub ent_status: String,
    pub search: String,
    pub sort: usize,
    pub active: bool,
    pub rotating: bool,
    pub hit_valid: bool,
    pub selection: Option<String>,
    pub scale: f32,
    pub angles: [f32; 3],
    pub origin: [f32; 3],
    pub bounds: Option<([f32; 3], [f32; 3])>,
    pub placed: Vec<ModelPlacement>,
    pub placed_draws: Arc<Vec<DynamicModelSurface>>,
    pub preview_draws: Arc<Vec<DynamicModelSurface>>,
    pub placed_dirty: bool,
    pub preview_key: Option<ModelPlacement>,
    pub map_label: String,
    pub status: String,
    pub thumbnails: HashMap<String, egui::TextureHandle>,
    pub thumbnails_pending: HashSet<String>,
    pub thumbnails_missing: HashSet<String>,
}

impl Default for ModelPlacementUi {
    fn default() -> Self {
        Self {
            kind: PlacementKind::Md3, bmodel_search: String::new(),
            bmodel_sort: 0, bmodel_selection: None,
            bmodel_pivot: [0.0; 3], bmodel_bounds: None,
            bmodel_placed: Vec::new(),
            bmodel_collision_dirty: true, bmodel_collision_local: false,
            ent_filename: String::new(), ent_overwrite: false, ent_status: String::new(),
            search: String::new(), sort: 0, active: false, rotating: false,
            hit_valid: false, selection: None, scale: 1.0, angles: [0.0; 3],
            origin: [0.0; 3], bounds: None, placed: Vec::new(),
            placed_draws: Arc::new(Vec::new()), preview_draws: Arc::new(Vec::new()),
            placed_dirty: false, preview_key: None, map_label: String::new(),
            status: String::new(), thumbnails: HashMap::new(),
            thumbnails_pending: HashSet::new(), thumbnails_missing: HashSet::new(),
        }
    }
}

/// This is the exact MD3 `angles_to_axis` transform from the CGame presenter.
pub(super) fn model_axis(angles: [f32; 3], scale: f32) -> [[f32; 3]; 3] {
    let (sp, cp) = angles[0].to_radians().sin_cos();
    let (sy, cy) = angles[1].to_radians().sin_cos();
    let (sr, cr) = angles[2].to_radians().sin_cos();
    let forward = [cp * cy, cp * sy, -sp];
    let right = [-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp];
    let up = [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp];
    [forward, [-right[0], -right[1], -right[2]], up].map(|v| v.map(|c| c * scale))
}

fn placement_corners(placement: &ModelPlacement) -> [[f32; 3]; 8] {
    let (mins, maxs) = placement.bounds;
    let axis = model_axis(placement.angles, placement.scale);
    std::array::from_fn(|i| {
        let v = [if i & 1 == 0 { mins[0] } else { maxs[0] },
                 if i & 2 == 0 { mins[1] } else { maxs[1] },
                 if i & 4 == 0 { mins[2] } else { maxs[2] }];
        std::array::from_fn(|j| placement.origin[j]
            + axis[0][j] * v[0] + axis[1][j] * v[1] + axis[2][j] * v[2])
    })
}

fn bmodel_corners(placement: &BmodelPlacement) -> [[f32; 3]; 8] {
    let (mins, maxs) = placement.bounds;
    let pose = placement.instance();
    std::array::from_fn(|i| {
        let vertex = [if i & 1 == 0 { mins[0] } else { maxs[0] },
            if i & 2 == 0 { mins[1] } else { maxs[1] },
            if i & 4 == 0 { mins[2] } else { maxs[2] }];
        let r = pose.rotate(vertex);
        std::array::from_fn(|j| pose.origin[j] + r[j])
    })
}

/// BSP entity `origin` (if authored) is the preferred pivot. Otherwise use
/// the source model's compiled bounds midpoint, not the world's origin.
pub(super) fn bmodel_pivot(map: &scene::PreparedMap, model: u32) -> Option<([f32; 3], ([f32; 3], [f32; 3]))> {
    let bounds = *map.bsp_stats.as_ref()?.inline_model_bounds.get(model as usize)?;
    let midpoint = std::array::from_fn(|i| (bounds.0[i] + bounds.1[i]) * 0.5);
    let pivot = map.brush_entities.iter().find(|entity| entity.model == model)
        .and_then(|entity| entity.spawn_vars.iter().find(|(key, _)| key.eq_ignore_ascii_case("origin")))
        .and_then(|(_, value)| {
            let parts = value.split_whitespace()
                .map(str::parse::<f32>).collect::<Result<Vec<_>, _>>().ok()?;
            (parts.len() == 3 && parts.iter().all(|x| x.is_finite()))
                .then(|| [parts[0], parts[1], parts[2]])
        }).unwrap_or(midpoint);
    Some((pivot, bounds))
}

/// Project BSP triangles in a fixed isometric camera to paint small, cheap
/// geometry-derived thumbnails directly from the prepared inline mesh.
fn paint_bmodel_thumbnail(ui: &mut egui::Ui, map: &scene::PreparedMap,
    model: &scene::InlineModelGeometry, bounds: ([f32; 3], [f32; 3])) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 42.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, theme::CONTROL);
    let midpoint: [f32; 3] = std::array::from_fn(|i| (bounds.0[i] + bounds.1[i]) * 0.5);
    let span = (0..3).map(|i| (bounds.1[i] - bounds.0[i]).abs())
        .fold(1.0_f32, f32::max);
    let project = |v: [f32; 3]| {
        let p = scene::jka_position(v);
        let x = (p[0] - midpoint[0]) / span;
        let y = (p[1] - midpoint[1]) / span;
        let z = (p[2] - midpoint[2]) / span;
        rect.center() + egui::vec2((x - y * 0.70) * 34.0,
            (-z * 0.80 - x * 0.30 - y * 0.28) * 34.0)
    };
    let start = model.vertices.start as usize;
    let end = (model.vertices.end as usize).min(map.inline_vertices.len());
    if start >= end { return; }
    let vertices = &map.inline_vertices[start..end];
    let tri_count = vertices.len() / 3;
    let step = (tri_count / 55).max(1);
    for tri in vertices.chunks_exact(3).step_by(step).take(55) {
        let points = [project(tri[0].position), project(tri[1].position), project(tri[2].position)];
        for k in 0..3 {
            ui.painter().line_segment([points[k], points[(k + 1) % 3]],
                egui::Stroke::new(0.65, theme::TEXT_DIM));
        }
    }
}

impl App {
    pub(in crate::app) fn open_model_placement_picker(&mut self) {
        if self.prepared_map_cache.is_none() || self.map_collision.is_none() {
            self.console_status = "MODEL PLACEMENT: LOAD A MAP FIRST".into();
            self.publish_ui();
            return;
        }
        self.model_placement.map_label = self.prepared_map_cache.as_ref()
            .map(|map| map.label.clone()).unwrap_or_default();
        self.model_placement.kind = PlacementKind::Md3;
        self.model_placement.active = false;
        self.model_placement.rotating = false;
        self.model_placement.preview_draws = Arc::new(Vec::new());
        self.model_placement.preview_key = None;
        self.ensure_asset_viewer_catalog();
        self.set_overlay(OverlayMode::ModelPicker);
    }

    pub(in crate::app) fn open_bmodel_placement_picker(&mut self) {
        if self.prepared_map_cache.is_none() || self.map_collision.is_none() {
            self.console_status = "BMODEL PLACEMENT: LOAD A BSP MAP FIRST".into();
            self.publish_ui();
            return;
        }
        self.model_placement.map_label = self.prepared_map_cache.as_ref()
            .map(|map| map.label.clone()).unwrap_or_default();
        self.model_placement.kind = PlacementKind::Bmodel;
        self.model_placement.active = false;
        self.model_placement.rotating = false;
        self.model_placement.preview_draws = Arc::new(Vec::new());
        self.model_placement.preview_key = None;
        self.model_placement.hit_valid = false;
        self.set_overlay(OverlayMode::BmodelPicker);
    }

    pub(in crate::app) fn choose_placement_bmodel(&mut self, model: u32) {
        let Some(cache) = self.prepared_map_cache.as_ref() else { return; };
        let Some((pivot, bounds)) = bmodel_pivot(&cache.map, model) else {
            self.model_placement.status = format!("*{model}: missing bounds");
            return;
        };
        if !cache.map.inline_models.iter().any(|entry| entry.model == model) {
            self.model_placement.status = format!("*{model}: no drawable surfaces");
            return;
        }
        self.model_placement.kind = PlacementKind::Bmodel;
        self.model_placement.bmodel_selection = Some(model);
        self.model_placement.bmodel_pivot = pivot;
        self.model_placement.bmodel_bounds = Some(bounds);
        self.model_placement.scale = 1.0;
        self.model_placement.angles = [0.0; 3];
        self.model_placement.hit_valid = false;
        self.model_placement.rotating = false;
        self.model_placement.active = true;
        self.model_placement.status.clear();
        // Grow the model's private GPU slot once at selection, not every frame.
        self.ensure_inline_instance_capacity();
        self.set_overlay(OverlayMode::None);
    }

    pub(in crate::app) fn choose_placement_model(&mut self, qpath: String) {
        if let Err(error) = self.ensure_asset_preview_entity_presenter() {
            self.model_placement.status = error;
            return;
        }
        let bounds = self.asset_preview_entity_presenter.as_mut()
            .expect("presenter initialized").md3_frame_bounds(&qpath);
        let bounds = match bounds {
            Ok(bounds) => bounds,
            Err(error) => {
                self.model_placement.status = format!("{qpath}: {error}");
                return;
            }
        };
        self.model_placement.kind = PlacementKind::Md3;
        self.model_placement.selection = Some(qpath);
        self.model_placement.scale = 1.0;
        self.model_placement.angles = [0.0; 3];
        self.model_placement.bounds = Some(bounds);
        self.model_placement.active = true;
        self.model_placement.rotating = false;
        self.model_placement.preview_key = None;
        self.model_placement.status.clear();
        self.set_overlay(OverlayMode::None);
    }

    pub(in crate::app) fn cancel_placement(&mut self) {
        self.model_placement.active = false;
        self.model_placement.rotating = false;
        self.model_placement.hit_valid = false;
        self.model_placement.preview_key = None;
        self.model_placement.preview_draws = Arc::new(Vec::new());
        self.egui_repaint_requested = true;
    }

    pub(in crate::app) fn stamp_placement(&mut self) {
        if !self.model_placement.active || !self.model_placement.hit_valid { return; }
        if self.model_placement.kind == PlacementKind::Bmodel {
            if let (Some(model), Some(bounds)) = (self.model_placement.bmodel_selection,
                self.model_placement.bmodel_bounds) {
                self.model_placement.bmodel_placed.push(BmodelPlacement {
                    model, bounds, pivot: self.model_placement.bmodel_pivot,
                    hit: self.model_placement.origin,
                    angles: self.model_placement.angles,
                });
                self.model_placement.bmodel_collision_dirty = true;
                self.model_placement.status = format!("{} BSP model(s) placed",
                    self.model_placement.bmodel_placed.len());
                self.ensure_inline_instance_capacity();
            }
            return;
        }
        let (Some(qpath), Some(bounds)) = (
            self.model_placement.selection.clone(), self.model_placement.bounds) else { return; };
        self.model_placement.placed.push(ModelPlacement {
            qpath, origin: self.model_placement.origin,
            angles: self.model_placement.angles, scale: self.model_placement.scale, bounds,
        });
        self.model_placement.placed_dirty = true;
        self.model_placement.status = format!("{} model(s) placed", self.model_placement.placed.len());
    }

    pub(in crate::app) fn undo_model_placement(&mut self) {
        if self.model_placement.kind == PlacementKind::Bmodel {
            if self.model_placement.bmodel_placed.pop().is_some() {
                self.model_placement.bmodel_collision_dirty = true;
                self.model_placement.status = "Last BMODEL placement undone".into();
            }
            return;
        }
        if self.model_placement.placed.pop().is_some() {
            self.model_placement.placed_dirty = true;
            self.model_placement.status = "Last placement undone".into();
        }
    }

    /// Keep exact CM_TransformedBoxTrace/CM_TransformedPointContents copies
    /// in the in-process server and the client's shared collision world.
    /// Never change collision on a remote server: a local visual map mod is
    /// not an authoritative server entity and would desync prediction.
    fn sync_bmodel_collision(&mut self) {
        let local = self.local_server.is_some();
        if !self.model_placement.bmodel_collision_dirty
            && self.model_placement.bmodel_collision_local == local { return; }
        let Some(world) = self.map_collision.as_ref() else { return; };
        let placed = if local {
            self.model_placement.bmodel_placed.iter().filter_map(|p| {
                let instance = p.instance();
                i32::try_from(p.model).ok().map(|index| jka_movement::PlacedInlineCollision {
                    index, origin: instance.origin, angles: p.angles,
                })
            }).collect()
        } else {
            Vec::new()
        };
        world.set_placed_inline_models(placed);
        self.model_placement.bmodel_collision_local = local;
        self.model_placement.bmodel_collision_dirty = false;
    }

    /// Called before the RenderSnapshot is published. Model vertices for fixed
    /// placements are built only on a stamp/undo, not once per game frame.
    pub(in crate::app) fn update_model_placement(&mut self) {
        let label = self.prepared_map_cache.as_ref().map(|map| map.label.as_str()).unwrap_or("");
        if self.model_placement.map_label != label {
            self.model_placement = ModelPlacementUi {
                map_label: label.to_owned(), ..ModelPlacementUi::default()
            };
        }
        self.sync_bmodel_collision();
        if self.model_placement.active && !self.model_placement.rotating {
            use jka_movement::TraceWorld;
            let start = scene::jka_position(self.camera.position.to_array());
            // Use the center camera ray, never weapon muzzle/dynamic-crosshair
            // offsets: the visual gizmo and actual insertion point must agree.
            let direction = scene::jka_position(self.camera.forward().to_array());
            let end = std::array::from_fn(|i| start[i] + direction[i] * 131_072.0);
            self.model_placement.hit_valid = false;
            if let Some(world) = self.map_collision.as_mut() {
                let trace = world.trace(jka_movement::TraceQuery {
                    start, end, mins: [0.0; 3], maxs: [0.0; 3],
                    pass_entity: -1, mask: 0x1 | 0x1000,
                });
                if trace.fraction < 1.0 && trace.start_solid == 0 {
                    self.model_placement.hit_valid = true;
                    self.model_placement.origin = trace.end;
                }
            }
        }
        if self.model_placement.kind == PlacementKind::Bmodel {
            self.model_placement.preview_draws = Arc::new(Vec::new());
            self.model_placement.preview_key = None;
            if !self.model_placement.placed_dirty { return; }
        }
        if !self.model_placement.active || !self.model_placement.hit_valid {
            self.model_placement.preview_draws = Arc::new(Vec::new());
            self.model_placement.preview_key = None;
        }
        if !self.model_placement.placed_dirty
            && (!self.model_placement.active || !self.model_placement.hit_valid
                || self.model_placement.selection.is_none()) {
            if !self.model_placement.active || !self.model_placement.hit_valid {
                self.model_placement.preview_draws = Arc::new(Vec::new());
                self.model_placement.preview_key = None;
            }
            return;
        }
        if let Err(error) = self.ensure_asset_preview_entity_presenter() {
            self.model_placement.status = error;
            return;
        }
        let presenter = self.asset_preview_entity_presenter.as_mut().unwrap();
        if self.model_placement.placed_dirty {
            let mut surfaces = Vec::new();
            for placement in &self.model_placement.placed {
                match presenter.present_static_md3(
                    1023, &placement.qpath, placement.origin,
                    model_axis(placement.angles, placement.scale), [1.0; 4], 0,
                ) {
                    Ok(draws) => surfaces.extend(draws),
                    Err(error) => self.model_placement.status = error,
                }
            }
            self.model_placement.placed_draws = Arc::new(surfaces);
            self.model_placement.placed_dirty = false;
        }
        if self.model_placement.kind == PlacementKind::Md3
            && self.model_placement.active && self.model_placement.hit_valid {
            let placement = ModelPlacement {
                qpath: self.model_placement.selection.as_ref().unwrap().clone(),
                origin: self.model_placement.origin,
                angles: self.model_placement.angles, scale: self.model_placement.scale,
                bounds: self.model_placement.bounds.unwrap(),
            };
            if self.model_placement.preview_key.as_ref() != Some(&placement) {
                match presenter.present_static_md3(
                    1023, &placement.qpath, placement.origin,
                    model_axis(placement.angles, placement.scale), [1.0; 4], 0,
                ) {
                    Ok(draws) => self.model_placement.preview_draws = Arc::new(draws),
                    Err(error) => {
                        self.model_placement.preview_draws = Arc::new(Vec::new());
                        self.model_placement.status = error;
                    }
                }
                self.model_placement.preview_key = Some(placement);
            }
        }
    }

    pub(super) fn egui_model_picker(&mut self, root: &mut egui::Ui) {
        self.ensure_asset_viewer_catalog();
        let mut selected = None;
        let mut close = false;
        let mut image_requests = Vec::new();
        let state = &mut self.model_placement;
        egui::Window::new("MD3 MODEL PLACEMENT")
            .id(egui::Id::new("md3_model_placement_catalog"))
            .default_width(690.0).default_height(560.0)
            .resizable(true).collapsible(false)
            .show(root.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.heading("PLACE MODEL");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("CLOSE").clicked() { close = true; }
                    });
                });
                ui.label("Choose any loaded MD3. Thumbnails show actual model geometry; the live world preview shows its authored materials.");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut state.search)
                        .hint_text("Filter by filename or folder...").desired_width(420.0));
                    egui::ComboBox::from_id_salt("model_sort")
                        .selected_text(match state.sort { 1 => "Z → A", 2 => "Folder", _ => "A → Z" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.sort, 0, "A → Z");
                            ui.selectable_value(&mut state.sort, 1, "Z → A");
                            ui.selectable_value(&mut state.sort, 2, "Folder");
                        });
                });
                if self.ui_catalog.pending.asset_viewer {
                    ui.label("Scanning loaded assets on the catalog worker...");
                    return;
                }
                if let Some(error) = &self.asset_viewer_catalog_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                    return;
                }
                let query = state.search.trim().to_ascii_lowercase();
                let mut models: Vec<_> = self.asset_viewer_entries.iter()
                    .filter(|entry| entry.kind == AssetKind::Md3)
                    .filter(|entry| query.is_empty() || entry.qpath.to_ascii_lowercase().contains(&query))
                    .collect();
                match state.sort {
                    1 => models.sort_unstable_by(|a,b| b.qpath.cmp(&a.qpath)),
                    2 => models.sort_unstable_by(|a,b| a.folder.cmp(&b.folder).then(a.leaf.cmp(&b.leaf))),
                    _ => models.sort_unstable_by(|a,b| a.qpath.cmp(&b.qpath)),
                }
                ui.label(format!("{} matching MD3s   •   {} already placed", models.len(), state.placed.len()));
                ui.separator();
                egui::ScrollArea::vertical().id_salt("model_placement_results")
                    .auto_shrink([false, false]).show_rows(ui, 46.0, models.len(), |ui, rows| {
                        for i in rows {
                            let entry = models[i];
                            ui.horizontal(|ui| {
                                if let Some(texture) = state.thumbnails.get(&entry.qpath) {
                                    ui.add(egui::Image::new(texture)
                                        .fit_to_exact_size(egui::vec2(34.0, 34.0)));
                                } else {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(34.0, 34.0), egui::Sense::hover());
                                    ui.painter().rect_filled(rect, 3.0, theme::CONTROL);
                                    if !state.thumbnails_missing.contains(&entry.qpath)
                                        && state.thumbnails_pending.insert(entry.qpath.clone()) {
                                        image_requests.push(entry.qpath.clone());
                                    }
                                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER,
                                        "MD3", egui::FontId::monospace(10.0), theme::TEXT_DIM);
                                }
                                if ui.button(&entry.qpath).clicked() {
                                    selected = Some(entry.qpath.clone());
                                }
                            });
                        }
                    });
                if !state.status.is_empty() { ui.label(&state.status); }
                ui.separator();
                ui.label("Place: left click · Size: wheel · Rotate: hold right click and move mouse · Undo: Ctrl+Z · Change model: Tab · Exit: Esc");
            });
        for qpath in image_requests {
            self.ui_catalog.request(&self.base, self.game.as_deref(),
                ui_catalog::CatalogRequest::ModelThumbnail { qpath });
        }
        if close { self.set_overlay(OverlayMode::None); }
        if let Some(qpath) = selected { self.choose_placement_model(qpath); }
    }

    /// Screen-space projected oriented bounds; coordinates originate in JKA
    /// model space, not a made-up AABB about the instance origin.
    pub(super) fn egui_model_placement_hud(&mut self, root: &mut egui::Ui) {
        if !self.model_placement.active { return; }
        let Some(window) = self.window.as_ref() else { return; };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 { return; }
        let state = &self.model_placement;
        let painter = root.painter();
        if state.hit_valid {
            let corners = match state.kind {
                PlacementKind::Md3 => match (&state.selection, state.bounds) {
                    (Some(qpath), Some(bounds)) => Some(placement_corners(&ModelPlacement {
                        qpath: qpath.clone(), origin: state.origin,
                        scale: state.scale, angles: state.angles, bounds,
                    })),
                    _ => None,
                },
                PlacementKind::Bmodel => match (state.bmodel_selection, state.bmodel_bounds) {
                    (Some(model), Some(bounds)) => Some(bmodel_corners(&BmodelPlacement {
                        model, pivot: state.bmodel_pivot, hit: state.origin,
                        angles: state.angles, bounds,
                    })),
                    _ => None,
                },
            };
            if let Some(corners) = corners {
                let pixels_per_point = root.ctx().pixels_per_point().max(0.001);
                let projected: [Option<egui::Pos2>; 8] = std::array::from_fn(|i| {
                    self.camera.project_to_screen(size.width, size.height,
                        glam::Vec3::from_array(scene::render_position(corners[i])))
                        .map(|point| egui::pos2(point.x / pixels_per_point, point.y / pixels_per_point))
                });
                let color = if state.rotating { egui::Color32::from_rgb(255, 179, 73) }
                    else { egui::Color32::from_rgb(76, 220, 191) };
                for i in 0..8 {
                    for bit in [1,2,4] {
                        if i & bit != 0 { continue; }
                        if let (Some(a), Some(b)) = (projected[i], projected[i | bit]) {
                            painter.line_segment([a,b], egui::Stroke::new(1.7, color));
                        }
                    }
                }
                if let Some(center) = self.camera.project_to_screen(size.width, size.height,
                    glam::Vec3::from_array(scene::render_position(state.origin))) {
                    painter.circle_filled(egui::pos2(center.x / pixels_per_point,
                        center.y / pixels_per_point), 3.0, color);
                }
            }
        }
        let bounds = root.max_rect();
        let top = bounds.min + egui::vec2(20.0, 20.0);
        painter.rect_filled(egui::Rect::from_min_size(top, egui::vec2(690.0, 78.0)),
            5.0, egui::Color32::from_black_alpha(188));
        painter.text(top + egui::vec2(12.0, 11.0), egui::Align2::LEFT_TOP,
            match state.kind {
                PlacementKind::Md3 => format!("MD3 PLACEMENT   {}", state.selection.as_deref().unwrap_or("")),
                PlacementKind::Bmodel => format!("BMODEL PLACEMENT   *{}", state.bmodel_selection.unwrap_or(0)),
            },
            egui::FontId::monospace(12.0), theme::TEXT);
        painter.text(top + egui::vec2(12.0, 34.0), egui::Align2::LEFT_TOP,
            format!("{}   Pitch {:.1}°   Yaw {:.1}°   Roll {:.1}°  {}",
                if state.kind == PlacementKind::Md3 { format!("Scale {:.3}×", state.scale) } else { "BMODEL 1:1 (scale locked)".into() },
                state.angles[0], state.angles[1], state.angles[2],
                if !state.hit_valid { "NO SURFACE" } else if state.rotating { "ROTATE" } else { "READY" }),
            egui::FontId::monospace(11.0), theme::ACCENT);
        painter.text(top + egui::vec2(12.0, 56.0), egui::Align2::LEFT_TOP,
            if state.kind == PlacementKind::Md3 {
                "LMB Place · Wheel Size · RMB Hold Pivot + Rotate · Shift Pitch · Alt Roll · Tab Browser · Ctrl+Z Undo · Esc Exit"
            } else {
                "LMB Place · RMB Rotate · Shift Pitch · Alt Roll · Tab Browser · Ctrl+Z Undo · Esc Exit (scale locked)"
            },
            egui::FontId::monospace(9.0), theme::TEXT_DIM);
    }
}

impl App {
    pub(super) fn egui_bmodel_picker(&mut self, root: &mut egui::Ui) {
        let Some(map) = self.prepared_map_cache.as_ref().map(|cache| Arc::clone(&cache.map)) else {
            return;
        };
        let mut picked = None;
        let mut close = false;
        let state = &mut self.model_placement;
        egui::Window::new("BMODEL PLACEMENT · BSP INLINE MODELS")
            .id(egui::Id::new("bsp_bmodel_placement_catalog"))
            .default_width(680.0).default_height(560.0)
            .resizable(true).collapsible(false).show(root.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.heading("PLACE BMODEL");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("CLOSE").clicked() { close = true; }
                    });
                });
                ui.label("Re-use any drawable BSP inline *N model from this map. Thumbnails are sampled wireframes of its actual geometry.");
                ui.small("Native brush collision is active in the local game. On remote servers these placements remain visual-only until loaded server-side.");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut state.bmodel_search)
                        .hint_text("Filter by *number or entity classname...").desired_width(420.0));
                    egui::ComboBox::from_id_salt("bmodel_sort")
                        .selected_text(if state.bmodel_sort == 1 { "Reverse number" } else { "Model number" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.bmodel_sort, 0, "Model number");
                            ui.selectable_value(&mut state.bmodel_sort, 1, "Reverse number");
                        });
                });
                let query = state.bmodel_search.trim().to_ascii_lowercase();
                let mut available: Vec<_> = map.inline_models.iter()
                    .filter(|model| model.model > 0)
                    .filter(|model| bmodel_pivot(&map, model.model).is_some())
                    .collect();
                available.sort_unstable_by_key(|model| model.model);
                available.dedup_by_key(|model| model.model);
                if state.bmodel_sort == 1 { available.reverse(); }
                let filtered: Vec<_> = available.into_iter().filter(|model| {
                    let label = format!("*{}", model.model);
                    label.contains(&query) || map.brush_entities.iter()
                        .filter(|entity| entity.model == model.model)
                        .flat_map(|entity| entity.spawn_vars.iter())
                        .any(|(key, value)| key.eq_ignore_ascii_case("classname")
                            && value.to_ascii_lowercase().contains(&query))
                }).collect();
                ui.label(format!("{} matching submodels   •   {} placed",
                    filtered.len(), state.bmodel_placed.len()));
                ui.separator();
                egui::ScrollArea::vertical().id_salt("bmodel_placement_results")
                    .auto_shrink([false, false]).show_rows(ui, 52.0, filtered.len(), |ui, rows| {
                        for index in rows {
                            let model = filtered[index];
                            let Some((pivot, bounds)) = bmodel_pivot(&map, model.model) else { continue; };
                            let classname = map.brush_entities.iter()
                                .find(|entity| entity.model == model.model)
                                .and_then(|entity| entity.spawn_vars.iter().find(|(key, _)| key.eq_ignore_ascii_case("classname")))
                                .map_or("(unclassified)", |(_, value)| value.as_str());
                            ui.horizontal(|ui| {
                                paint_bmodel_thumbnail(ui, &map, model, bounds);
                                ui.vertical(|ui| {
                                    if ui.button(format!("*{} · {classname}", model.model)).clicked() {
                                        picked = Some(model.model);
                                    }
                                    ui.small(format!("Origin: {:.0}, {:.0}, {:.0}  ·  {:.0} × {:.0} × {:.0} u",
                                        pivot[0], pivot[1], pivot[2],
                                        bounds.1[0] - bounds.0[0],
                                        bounds.1[1] - bounds.0[1],
                                        bounds.1[2] - bounds.0[2]));
                                });
                            });
                        }
                    });
                ui.separator();
                ui.label("Original BSP pivot is preserved. Translation/rotation use the pivot; BMODEL scaling is disabled for server compatibility.");
                ui.label("LMB Place · RMB Hold to Lock and Rotate · Tab Reopen · Ctrl+Z Undo · Esc Exit");
                if !state.status.is_empty() { ui.label(&state.status); }
            });
        if close { self.set_overlay(OverlayMode::None); }
        if let Some(model) = picked { self.choose_placement_bmodel(model); }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmodel_placement_cancels_original_world_pivot() {
        let placed = BmodelPlacement { model: 6,
            pivot: [1000.0, 100.0, 50.0], hit: [20.0, 30.0, 40.0],
            angles: [0.0; 3],
            bounds: ([990.0, 90.0, 40.0], [1010.0, 110.0, 60.0]),
        };
        let pose = placed.instance();
        let rotated = pose.rotate(placed.pivot);
        for axis in 0..3 {
            assert!((pose.origin[axis] + rotated[axis] - placed.hit[axis]).abs() < 0.001);
        }
        let rotated_placed = BmodelPlacement { angles: [20.0, 65.0, 45.0], ..placed };
        let pose = rotated_placed.instance();
        let rotated = pose.rotate(rotated_placed.pivot);
        for axis in 0..3 {
            assert!((pose.origin[axis] + rotated[axis] - rotated_placed.hit[axis]).abs() < 0.001);
        }
    }

    #[test]
    fn oriented_bounds_scale_about_model_origin() {
        let mut model = ModelPlacement {
            qpath: "models/map_objects/test.md3".into(),
            origin: [100.0, 20.0, -5.0], angles: [0.0; 3], scale: 1.0,
            bounds: ([-2.0, -4.0, -6.0], [2.0, 4.0, 6.0]),
        };
        let corners = placement_corners(&model);
        assert_eq!(corners[0], [98.0, 16.0, -11.0]);
        assert_eq!(corners[7], [102.0, 24.0, 1.0]);
        model.scale = 2.0;
        let doubled = placement_corners(&model);
        assert_eq!(doubled[0], [96.0, 12.0, -17.0]);
        assert_eq!(doubled[7], [104.0, 28.0, 7.0]);
    }
}
