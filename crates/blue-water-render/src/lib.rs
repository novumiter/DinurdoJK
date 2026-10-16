//! JKA host services used by the preserved standalone sky and cloud code.
pub use naga_oil::compose::ShaderDefValue;
use naga_oil::compose::{
    ComposableModuleDescriptor, Composer, NagaModuleDescriptor, ShaderLanguage, ShaderType,
};
use std::collections::HashMap;
#[path = "../../../vendor/blue-water-sea-and-sky/shared/render/src/aerial.rs"]
pub mod aerial;
pub use aerial::*;
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub type ShaderDefs = Vec<(String, ShaderDefValue)>;
pub type ShaderError = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone, Copy, Debug)]
pub struct ShaderFile {
    pub path: &'static str,
    pub source: &'static str,
}
/// Host packages supply their preserved source roots through their build scripts.
#[macro_export]
macro_rules! shader_file {
    ($path:literal) => {
        $crate::ShaderFile {
            path: concat!(env!("BLUE_WATER_SHADER_PACKAGE"), "/", $path),
            source: include_str!(concat!(env!("BLUE_WATER_SHADER_ROOT"), "/", $path)),
        }
    };
}
pub mod shaders {
    pub mod builtin {
        pub const MOTION: crate::ShaderFile = crate::ShaderFile {
            path: "naval/shaders/motion.wgsl",
            source: include_str!(
                "../../../vendor/blue-water-sea-and-sky/shared/render/shaders/motion.wgsl"
            ),
        };
        pub const AERIAL_VOLUME: crate::ShaderFile = crate::ShaderFile {
            path: "naval/shaders/aerial_volume.wgsl",
            source: include_str!(
                "../../../vendor/blue-water-sea-and-sky/shared/render/shaders/aerial_volume.wgsl"
            ),
        };
    }
}
#[derive(Clone)]
pub enum Pipeline {
    Compute(wgpu::ComputePipeline),
    Render(wgpu::RenderPipeline),
}
pub type PipelineId = usize;
pub struct PipelineBuild<'a> {
    pub device: &'a wgpu::Device,
    pub module: &'a wgpu::ShaderModule,
}
pub type PipelineRecipe = Box<dyn for<'a> Fn(PipelineBuild<'a>) -> Pipeline + Send + Sync>;
#[derive(Default)]
pub struct PipelineRegistry {
    pipelines: Vec<Pipeline>,
}
impl PipelineRegistry {
    pub fn compute(&self, id: PipelineId) -> &wgpu::ComputePipeline {
        match &self.pipelines[id] {
            Pipeline::Compute(p) => p,
            _ => panic!("expected compute pipeline"),
        }
    }
    pub fn render(&self, id: PipelineId) -> &wgpu::RenderPipeline {
        match &self.pipelines[id] {
            Pipeline::Render(p) => p,
            _ => panic!("expected render pipeline"),
        }
    }
}
pub fn compute_recipe(layouts: Vec<wgpu::BindGroupLayout>, entry: &'static str) -> PipelineRecipe {
    Box::new(move |build| {
        let layout = build
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &layouts.iter().map(Some).collect::<Vec<_>>(),
                immediate_size: 0,
            });
        Pipeline::Compute(
            build
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: build.module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
        )
    })
}
/// Optional upstream pipeline registration uses synchronous builds on JKA's device.
pub struct RenderContext<'a> {
    pub device: &'a wgpu::Device,
    registry: &'a mut PipelineRegistry,
    pub library: ShaderLibrary,
}
#[derive(Default)]
pub struct ShaderLibrary {
    files: Vec<ShaderFile>,
}
impl ShaderLibrary {
    pub fn text(&self, path: &str) -> Option<&str> {
        self.files.iter().find(|f| f.path == path).map(|f| f.source)
    }
}
impl<'a> RenderContext<'a> {
    pub fn new(device: &'a wgpu::Device, registry: &'a mut PipelineRegistry) -> Self {
        Self {
            device,
            registry,
            library: ShaderLibrary {
                files: vec![shaders::builtin::AERIAL_VOLUME, shaders::builtin::MOTION],
            },
        }
    }
    pub fn add_shader(&mut self, file: ShaderFile) -> Result<(), ShaderError> {
        if let Some(old) = self
            .library
            .files
            .iter_mut()
            .find(|old| old.path == file.path)
        {
            *old = file;
        } else {
            self.library.files.push(file);
        }
        Ok(())
    }
    pub fn add_pipeline(
        &mut self,
        label: &str,
        path: &str,
        defs: ShaderDefs,
        recipe: PipelineRecipe,
    ) -> Result<PipelineId, ShaderError> {
        let file = self
            .library
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(|| std::io::Error::other(format!("missing shader {path}")))?;
        let mut composer = Composer::default();
        let defs: HashMap<_, _> = defs.into_iter().collect();
        for file in &self.library.files {
            if !file.source.contains("#define_import_path") {
                continue;
            }
            let added = composer.add_composable_module(ComposableModuleDescriptor {
                source: file.source,
                file_path: file.path,
                language: ShaderLanguage::Wgsl,
                shader_defs: defs.clone(),
                ..Default::default()
            });
            if let Err(error) = added {
                return Err(std::io::Error::other(error.emit_to_string(&composer)).into());
            }
        }
        let module = composer
            .make_naga_module(NagaModuleDescriptor {
                source: file.source,
                file_path: file.path,
                shader_type: ShaderType::Wgsl,
                shader_defs: defs,
                ..Default::default()
            })
            .map_err(|error| std::io::Error::other(error.emit_to_string(&composer)))?;
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
            });
        let id = self.registry.pipelines.len();
        self.registry.pipelines.push(recipe(PipelineBuild {
            device: self.device,
            module: &module,
        }));
        Ok(id)
    }
}
#[derive(Clone)]
pub struct PlaceholderPipelines {
    pub compute: wgpu::ComputePipeline,
    pub render: wgpu::RenderPipeline,
}
#[derive(Clone, Copy)]
pub enum MakePipelines<'a> {
    Now,
    Deferred(&'a PlaceholderPipelines),
}
impl<'a> MakePipelines<'a> {
    pub fn is_now(self) -> bool {
        matches!(self, Self::Now)
    }
    pub fn from_placeholders(value: Option<&'a PlaceholderPipelines>) -> Self {
        value.map_or(Self::Now, Self::Deferred)
    }
    pub fn module(self, build: impl FnOnce() -> wgpu::ShaderModule) -> Option<wgpu::ShaderModule> {
        match self {
            Self::Now => Some(build()),
            Self::Deferred(_) => None,
        }
    }
    pub fn compute(self, build: impl FnOnce() -> wgpu::ComputePipeline) -> wgpu::ComputePipeline {
        match self {
            Self::Now => build(),
            Self::Deferred(p) => p.compute.clone(),
        }
    }
    pub fn render(self, build: impl FnOnce() -> wgpu::RenderPipeline) -> wgpu::RenderPipeline {
        match self {
            Self::Now => build(),
            Self::Deferred(p) => p.render.clone(),
        }
    }
}
pub enum UploadSink<'a> {
    Queue(&'a wgpu::Queue),
}
impl UploadSink<'_> {
    pub fn write_buffer(&mut self, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        match self {
            Self::Queue(queue) => queue.write_buffer(buffer, offset, bytes),
        }
    }
}
pub struct GpuTimer {
    queries: wgpu::QuerySet,
    labels: HashMap<String, u32>,
}
impl GpuTimer {
    pub fn new(device: &wgpu::Device, labels: &[&str]) -> Self {
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Blue Water timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: (labels.len().max(1) * 2) as u32,
        });
        Self {
            queries,
            labels: labels
                .iter()
                .enumerate()
                .map(|(i, label)| ((*label).into(), (i * 2) as u32))
                .collect(),
        }
    }
    pub fn compute_pass(&self, label: &str) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let index = *self.labels.get(label)?;
        Some(wgpu::ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(index),
            end_of_pass_write_index: Some(index + 1),
        })
    }
}
pub mod camera {
    pub const DEFAULT_FOV_Y_DEGREES: f32 = 52.0;
    pub const DEFAULT_FAR: f32 = 30_000.0;
    pub const HORIZON_FAR: f32 = 450_000.0;
    pub const EARTH_CURVATURE: f32 = 1.0 / (2.0 * 6_360_000.0);
    pub fn reversed_perspective(fov: f32, aspect: f32, near: f32, far: f32) -> glam::Mat4 {
        glam::Mat4::perspective_rh(fov, aspect, far, near)
    }
}
/// The preserved camera.wgsl block, including curvature and temporal fields.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniforms {
    pub view: [f32; 16],
    pub projection: [f32; 16],
    pub view_projection: [f32; 16],
    pub inverse_view: [f32; 16],
    pub inverse_projection: [f32; 16],
    pub inverse_view_projection: [f32; 16],
    pub position: [f32; 3],
    pub time: f32,
    pub viewport: [f32; 4],
    pub near: f32,
    pub far: f32,
    pub tan_half_fov_y: f32,
    pub aspect: f32,
    pub dt: f32,
    pub frame: u32,
    pub curvature: f32,
    pub padding: u32,
}

/// Camera facade used by the original cloud cascade tests.
pub struct Camera {
    pub eye: glam::Vec3,
    pub target: glam::Vec3,
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
    pub curvature: f32,
}
impl Camera {
    pub fn view(&self) -> glam::Mat4 {
        glam::Mat4::look_at_rh(self.eye, self.target, glam::Vec3::Y)
    }
    pub fn projection(&self, aspect: f32) -> glam::Mat4 {
        camera::reversed_perspective(self.fov_y_degrees.to_radians(), aspect, self.near, self.far)
    }
}
