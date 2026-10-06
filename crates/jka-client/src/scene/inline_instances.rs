//! Each server brush entity needs private render geometry, even when it shares
//! its collision/model handle with another misc_bsp or door.
use super::PreparedMap;
use std::collections::BTreeMap;

impl PreparedMap {
    pub(crate) fn inline_instance_capacity(&self) -> BTreeMap<u32, usize> {
        let mut counts = BTreeMap::new();
        for model in &self.inline_models {
            *counts.entry(model.model).or_default() += 1;
        }
        counts
    }

    /// Grow render slots only; collision handles and BSP metadata stay shared.
    /// Called at map upload, and on the cold path when a server adds instances.
    pub(crate) fn reserve_inline_instances(&mut self, wanted: &BTreeMap<u32, usize>) -> bool {
        let capacity = self.inline_instance_capacity();
        let mut grew = false;
        for (&id, &count) in wanted {
            let Some(&have) = capacity.get(&id) else {
                continue;
            };
            if count <= have {
                continue;
            }
            let source = self
                .inline_models
                .iter()
                .find(|model| model.model == id)
                .unwrap()
                .clone();
            let vertices = self.inline_vertices
                [source.vertices.start as usize..source.vertices.end as usize]
                .to_vec();
            let batches = self.inline_batches[source.batches.clone()].to_vec();
            for _ in have..count {
                let start = u32::try_from(self.inline_vertices.len())
                    .expect("inline vertex count exceeds u32");
                let end = start
                    .checked_add(u32::try_from(vertices.len()).unwrap())
                    .expect("inline vertex count exceeds u32");
                let batch_start = self.inline_batches.len();
                self.inline_vertices.extend_from_slice(&vertices);
                for mut batch in batches.iter().cloned() {
                    batch.vertices = start + (batch.vertices.start - source.vertices.start)
                        ..start + (batch.vertices.end - source.vertices.start);
                    self.inline_batches.push(batch);
                }
                self.inline_models.push(super::InlineModelGeometry {
                    model: id,
                    vertices: start..end,
                    batches: batch_start..self.inline_batches.len(),
                });
                grew = true;
            }
        }
        grew
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{AssetSearchPath, MapJobPool, MapPrepareOptions, MapSource};
    use jka_protocol::{
        demo::DemoReader,
        server::{Decoder, Event},
    };

    #[test]
    #[ignore = "requires JKA_TEST_BASE with stock assets and demos/ahfucker.dm_26"]
    fn ahfucker_shared_academy_instances_have_geometry_and_collision() {
        let base =
            std::path::PathBuf::from(std::env::var_os("JKA_TEST_BASE").expect("set JKA_TEST_BASE"));
        let mut assets = AssetSearchPath::open(&base).unwrap();
        let bytes = assets
            .read("demos/ahfucker.dm_26", 64 * 1024 * 1024)
            .unwrap()
            .unwrap()
            .bytes;
        let mut reader = DemoReader::new(std::io::Cursor::new(bytes));
        let mut decoder = Decoder::new();
        let mut names = Vec::new();
        let mut holders = BTreeMap::new();
        let mut teleported = false;
        let mut snapshots = 0;
        while let Some(record) = reader.next_record().unwrap() {
            let packet = decoder
                .parse_packet(record.sequence, &record.payload)
                .unwrap();
            for event in packet.events {
                match event {
                    Event::Gamestate { .. } => {
                        assert_eq!(decoder.map_name().as_deref(), Some("mp/ffa3"));
                        names = crate::scene::server_bsp_names(&decoder.configstrings).unwrap();
                    }
                    Event::Snapshot { .. } => {
                        let snapshot = decoder.latest_snapshot().unwrap();
                        snapshots += 1;
                        teleported |=
                            snapshot.player_state.field_f32("origin[0]").unwrap_or(0.0) > 50_000.0;
                        for entity in &snapshot.entities {
                            if entity.field_i32("eType") == Some(crate::cgame::ET_MOVER)
                                && entity.field_i32("solid") == Some(0x00ff_ffff)
                                && entity.field_i32("modelindex") == Some(9)
                            {
                                holders.insert(
                                    entity.number,
                                    ["pos.trBase[0]", "pos.trBase[1]", "pos.trBase[2]"]
                                        .map(|field| entity.field_f32(field).unwrap()),
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        assert!(teleported);
        assert_eq!(
            names,
            [
                "maps/academy4.bsp",
                "maps/mp/duel4.bsp",
                "maps/t1_inter.bsp",
                "maps/t2_rancor.bsp"
            ]
        );
        assert_eq!(holders.len(), 2);
        assert_eq!(holders[&56], [49805.0, -41942.0, -4245.0]);
        let jobs = MapJobPool::new_with_progress(|_| {}).unwrap();
        let mut map = crate::scene::prepare_source_with_sub_bsps(
            &base,
            None,
            &MapSource::Bsp("mp/ffa3".into()),
            &jobs,
            MapPrepareOptions {
                picmip: 2,
                grass: false,
                voxel_probe_gi: false,
                ocean: false,
                pbr_materials: false,
                planar_reflections: false,
                planar_environment: false,
                ..Default::default()
            },
            None,
            &names,
        )
        .unwrap();
        let model_count = map.bsp_stats.as_ref().unwrap().inline_model_bounds.len();
        assert!(map.reserve_inline_instances(&BTreeMap::from([(9, holders.len())])));
        let copies: Vec<_> = map
            .inline_models
            .iter()
            .filter(|model| model.model == 9)
            .collect();
        assert_eq!(copies.len(), 2);
        assert!(!copies[0].vertices.is_empty());
        assert!(copies[1].vertices.start >= copies[0].vertices.end);
        assert!(copies.iter().all(|copy| !copy.batches.is_empty()));
        assert_eq!(
            map.bsp_stats.as_ref().unwrap().inline_model_bounds.len(),
            model_count
        );
        let mut world = map.collision.take().unwrap();
        let query = jka_movement::TraceQuery {
            start: [51212.0, -42283.0, -3456.0],
            end: [51212.0, -42283.0, -3520.0],
            mins: [-15.0, -15.0, -24.0],
            maxs: [15.0, 15.0, 32.0],
            pass_entity: jka_movement::ENTITY_NONE,
            mask: 1 | 0x10000,
        };
        let distant = world.trace_entity(
            query,
            jka_movement::EntityClip::InlineModel {
                index: 9,
                origin: holders[&56],
                angles: [0.0; 3],
            },
        );
        let first = world.trace_entity(
            query,
            jka_movement::EntityClip::InlineModel {
                index: 9,
                origin: holders[&42],
                angles: [0.0; 3],
            },
        );
        assert!(
            distant.fraction < 1.0,
            "teleport destination has no imported BSP ground"
        );
        assert_eq!(first.fraction, 1.0);
        println!("AHFUCKER REGRESSION snapshots={snapshots} sharedModel=9 instances={} verticesPerInstance={} distantGroundFraction={}", holders.len(), copies[0].vertices.len(), distant.fraction);
    }
}
