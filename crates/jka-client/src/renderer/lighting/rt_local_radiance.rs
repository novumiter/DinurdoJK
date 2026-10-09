//! Independently traced finite-emitter HDR irradiance for clustered local RT.
//! Shares the variance-guided spatial/temporal filter, but not the sun cache or TAA.
use super::*;

#[cfg(test)]
#[path = "rt_local_receiver_tests.rs"]
mod receiver_tests;

pub(super) struct RtLocalRadiance {
    pub filter: rt_radiance::RtSunRadiance,
    _raw_texture: wgpu::Texture,
    pub raw: wgpu::TextureView,
    _reference_texture: wgpu::Texture,
    pub reference: wgpu::TextureView,
    dimensions: [u32; 2],
    pub enabled: bool,
    trace: Option<TraceResources>,
}
struct TraceResources {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    input: Option<wgpu::BindGroup>,
}

pub(super) fn receiver_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
    [unfilterable_texture_entry_stages(21, wgpu::ShaderStages::FRAGMENT),
     unfilterable_texture_entry_stages(22, wgpu::ShaderStages::FRAGMENT),
     unfilterable_texture_entry_stages(23, wgpu::ShaderStages::FRAGMENT)]
}

impl RtLocalRadiance {
    pub fn new(device: &wgpu::Device) -> Self {
        let (raw_texture, raw) = rt_radiance::texture(device, "RT local raw fallback", [1, 1], wgpu::TextureFormat::Rgba16Float);
        let (reference_texture, reference) = rt_radiance::texture(device, "RT local reference fallback", [1, 1], wgpu::TextureFormat::Rgba16Float);
        Self {
            filter: rt_radiance::RtSunRadiance::new(device),
            _raw_texture: raw_texture, raw,
            _reference_texture: reference_texture, reference,
            dimensions: [1, 1], enabled: false, trace: None,
        }
    }
    pub fn disable(&mut self) { self.enabled = false; self.filter.disable(); }
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self, device: &wgpu::Device, queue: &wgpu::Queue,
        dims: [u32; 2], view_proj: Mat4, eye: Vec3, previous_eye: Vec3,
        mode: u32, frame: u32, camera_cut: bool,
        depth: &wgpu::TextureView, motion: &wgpu::TextureView,
    ) -> bool {
        if mode == 0 { self.disable(); return false; }
        let changed = self.dimensions != dims;
        if changed {
            self.dimensions = dims;
            let (raw_texture, raw) = rt_radiance::texture(device, "RT local HDR stochastic raw", dims, wgpu::TextureFormat::Rgba16Float);
            self._raw_texture = raw_texture;
            self.raw = raw;
            let (reference_texture, reference) = rt_radiance::texture(device, "RT local HDR unshadowed reference", dims, wgpu::TextureFormat::Rgba16Float);
            self._reference_texture = reference_texture;
            self.reference = reference;
            if let Some(trace) = self.trace.as_mut() { trace.input = None; }
            self.filter.invalidate();
        }
        let filter_changed = self.filter.prepare(
            device, queue, dims, view_proj, eye, previous_eye, [0.0; 4],
            mode, frame, camera_cut, &self.raw, depth, motion, true,
        );
        self.enabled = true;
        changed || filter_changed
    }
    pub fn encode(
        &mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder,
        camera_layout: &wgpu::BindGroupLayout, camera: &wgpu::BindGroup,
        scene_layout: &wgpu::BindGroupLayout, scene: &wgpu::BindGroup,
        world_lights: &wgpu::Buffer, clusters: &wgpu::Buffer,
        lighting: &wgpu::Buffer, depth: &wgpu::TextureView, mode: u32,
    ) {
        if !self.enabled { return; }
        if self.trace.is_none() {
            self.trace = Some(TraceResources::new(device, camera_layout, scene_layout));
        }
        let trace = self.trace.as_mut().unwrap();
        if trace.input.is_none() {
            trace.input = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RT finite local irradiance inputs"), layout: &trace.layout,
                entries: &[
                    rt_radiance::bind_buffer(0, &self.filter.params),
                    rt_radiance::bind_texture(1, depth),
                    rt_radiance::bind_buffer(2, world_lights),
                    rt_radiance::bind_buffer(3, clusters),
                    rt_radiance::bind_buffer(4, lighting),
                    rt_radiance::bind_texture(5, &self.raw),
                    rt_radiance::bind_texture(6, &self.reference),
                ],
            }));
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RT stochastic finite local lighting"), timestamp_writes: None,
        });
        pass.set_pipeline(&trace.pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, trace.input.as_ref().unwrap(), &[]);
        pass.set_bind_group(2, scene, &[]);
        pass.dispatch_workgroups(self.dimensions[0].div_ceil(8), self.dimensions[1].div_ceil(8), 1);
        drop(pass);
        self.filter.encode(encoder, mode);
    }
}

impl TraceResources {
    fn new(device: &wgpu::Device, camera_layout: &wgpu::BindGroupLayout, scene_layout: &wgpu::BindGroupLayout) -> Self {
        let sampled = |binding| unfilterable_texture_entry_stages(binding, wgpu::ShaderStages::COMPUTE);
        let read_storage = |binding| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::COMPUTE, count: None,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false, min_binding_size: None },
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RT finite local irradiance layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::COMPUTE), sampled(1),
                read_storage(2), read_storage(3), uniform_entry(4, wgpu::ShaderStages::COMPUTE),
                rt_radiance::storage(5, wgpu::TextureFormat::Rgba16Float),
                rt_radiance::storage(6, wgpu::TextureFormat::Rgba16Float)],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RT transient local radiance ray queries"),
            source: wgpu::ShaderSource::Wgsl(source().expect("RT local radiance shader anchors").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RT local irradiance pipeline"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout), Some(scene_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RT local irradiance"), layout: Some(&pipeline_layout), module: &shader,
            entry_point: Some("main"), compilation_options: Default::default(), cache: None,
        });
        Self { layout, pipeline, input: None }
    }
}

pub(super) fn source() -> Result<String, String> {
    let donor = ray_traced_world_shader_source(include_str!("../../bsp.wgsl"))?;
    let declaration = rt_models::declaration;
    let mut shader = String::from("enable wgpu_ray_query;\n");
    for name in ["Camera", "PointLight", "ClusterRecord", "LightingSettings"] {
        shader.push_str(declaration(&donor, &format!("struct {name} "))?);
        shader.push_str(";\n");
    }
    shader.push_str("struct VertexOut { clip_position: vec4<f32>, world_position: vec3<f32> };\n");
    // Consume exactly the same alpha-tested BLAS/TLAS hit logic as the receiver.
    let start = donor.find("@group(3) @binding(8)").ok_or("RT scene binding missing")?;
    let last = donor.find("@group(3) @binding(14)").ok_or("RT scene alpha binding missing")?;
    let end = last + donor[last..].find(';').ok_or("RT scene binding terminator missing")? + 1;
    shader.push_str(&donor[start..end].replace("@group(3)", "@group(2)"));
    shader.push_str("\nconst ENABLE_CLUSTERED_LITE_DLIGHTS: bool = false;\n");
    shader.push_str("const CLUSTER_X: u32 = 16u;\nconst CLUSTER_Y: u32 = 9u;\nconst CLUSTER_Z: u32 = 24u;\nconst CLUSTER_NEAR: f32 = 1.0;\nconst CLUSTER_FAR: f32 = 65536.0;\n");
    for name in [
        "rt_rand_f", "rt_rand_vec2f", "rt_sample_count", "rt_local_sample_count", "rt_sample_local_emitter",
        "rt_alpha_repeat_coord", "rt_rgba_texel", "rt_sample_rgba", "rt_sample_alpha", "rt_alpha_generated_uv",
        "rt_alpha_candidate_blocks", "rt_candidate_glass_throughput", "rt_trace_shadow_query",
        "rt_trace_shadow_visibility", "rt_trace_local_visibility",
        "emitter_visibility", "local_light_attenuation", "cluster_for_fragment",
    ] {
        if name == "rt_candidate_glass_throughput"
            && !crate::renderer::lighting::rt_diagnostics::config().receiver.glass {
            continue;
        }
        shader.push_str(declaration(&donor, &format!("fn {name}("))?);
        shader.push('\n');
    }
    shader.push_str(include_str!("../../rt_local_radiance.wgsl"));
    Ok(shader)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_emitter_source_reuses_authoritative_rt_query_and_sampling() {
        let shader = source().expect("all receiver-source anchors must exist");
        let module = wgpu::naga::front::wgsl::parse_str(&shader)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&shader)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all()).validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&shader)));
        assert!(shader.contains("fn rt_sample_local_emitter("));
        assert!(shader.contains("fn rt_trace_local_visibility("));
        assert!(shader.contains("fn rt_alpha_candidate_blocks("));
        assert!(shader.contains("fn cluster_for_fragment("));
        assert!(shader.contains("@group(2) @binding(8) var rt_shadow_scene"));
        assert!(shader.contains("textureStore(local_raw,"));
        assert!(shader.contains("textureStore(local_reference,"));
    }

    #[test]
    fn renderer_local_fallback_preserves_direct_queries() {
        let receiver = include_str!("../../rt_resolution_read.wgsl");
        assert!(receiver.contains("abs(selected - f32(light_index + 1u))"));
        assert!(receiver.contains("local_light_shadow_visibility(light, input.world_position, normal)"));
        assert!(receiver.contains("rt_filtered_local"));
        let temporal = include_str!("../../rt_radiance_temporal.wgsl");
        let spatial = include_str!("../../rt_radiance_atrous.wgsl");
        assert!(temporal.contains("signal_id > 0.0"));
        assert!(spatial.contains("moment_data.z"));
        let generated = ray_traced_world_shader_source(include_str!("../../bsp.wgsl"))
            .expect("RT material source transforms must succeed");
        let helper = generated.split("fn rt_denoised_local_visibility(").nth(1)
            .expect("RT material receiver helper must be appended");
        assert!(helper.contains("return local_light_shadow_visibility(light, input.world_position, normal);"),
            "direct-ray fallback must not be rewritten into recursion");
        assert!(!helper.contains("rt_denoised_local_visibility(light, input, normal, cluster.indices[i])"),
            "RT direct-ray fallback must not refer to out-of-scope cluster loop variables");
    }
}
