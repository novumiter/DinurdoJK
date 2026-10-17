//! Reject JKA temporal history only where transient weather actually draws.
pub struct WeatherReactive(pub [wgpu::RenderPipeline; 3]);
impl WeatherReactive {
    pub fn new(device: &wgpu::Device, layouts: &[wgpu::BindGroupLayout; 2]) -> Self {
        let targets = [Some(wgpu::TextureFormat::Rg16Float.into())];
        let pipelines = [
            (
                crate::shaders::RAIN_WGSL,
                "rain_vertex",
                "rain_fragment",
                "RainVertex",
            ),
            (
                crate::shaders::SPLASHES_WGSL,
                "splash_vertex",
                "splash_fragment",
                "SplashVertex",
            ),
            (
                crate::shaders::BOLT_WGSL,
                "bolt_vertex",
                "bolt_fragment",
                "BoltVertex",
            ),
        ]
        .map(|(source, vertex, fragment, input)| {
            let source = source.replace("\r\n", "\n");
            let original =
                format!("@fragment\nfn {fragment}(in: {input}) -> @location(0) vec4<f32>");
            assert_eq!(source.matches(&original).count(), 1);
            let helper = format!("fn {fragment}(in: {input}) -> vec4<f32>");
            let source = format!(
                "{}\n@fragment fn weather_reactive(in: {input}) -> @location(0) vec2<f32> {{
                let color = {fragment}(in);
                if max(max(color.r, color.g), max(color.b, color.a)) < 0.01 {{ discard; }}
                // An out-of-frame history coordinate selects the current sample.
                return vec2<f32>(2.0, 2.0);
            }}",
                source.replace(&original, &helper)
            );
            let shader = crate::shaders::shader_module(
                device,
                &source,
                "JKA weather temporal rejection",
                &[],
            );
            let attributes = [wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 0,
            }];
            let end = [wgpu::VertexAttribute {
                shader_location: 1,
                ..attributes[0]
            }];
            let layout = |attributes| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes,
                })
            };
            let buffers = if input == "BoltVertex" {
                vec![layout(&attributes), layout(&end)]
            } else {
                vec![layout(&attributes)]
            };
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("JKA weather reactive"),
                bind_group_layouts: &[None, Some(&layouts[0]), Some(&layouts[1])],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("JKA weather temporal rejection"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("weather_reactive"),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        });
        Self(pipelines)
    }
}
