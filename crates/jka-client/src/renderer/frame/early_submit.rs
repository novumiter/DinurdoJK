//! Frame early submit.
//!
//! Without it, `render()` waits for a swapchain image before recording a single
//! pass, because the world pass draws straight into that image. The GPU then
//! idles through all of the frame's CPU recording, finish and submit. With
//! r_earlySubmit the frame renders into an offscreen copy of the swapchain
//! format and is submitted first; only then does the CPU wait for the image and
//! submit one copy into it. Late-latched view samples reach the GPU sooner too:
//! the image wait no longer sits between the latch and the submit.
use crate::renderer::{block_on, FrameInfo, Instant, RenderError, Renderer};

pub(in crate::renderer) struct EarlyPresentTarget {
    /// Also the screenshot source on early-submit frames.
    pub(in crate::renderer) texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Renderer {
    /// True when this frame's final image is produced entirely by the main frame
    /// encoder. Menus, CRT, SMAA and HDR output all touch the swapchain image
    /// after that encoder, so those frames keep the acquire-first order.
    /// Screenshots copy from the offscreen frame inside the main encoder.
    pub(in crate::renderer) fn early_submit_frame(&self, menu_backdrop_active: bool) -> bool {
        self.early_submit_enabled && self.offscreen_present_eligible(menu_backdrop_active)
    }

    /// r_presentCopy: TaystJK's Vulkan structure. Render into an offscreen image
    /// and copy it into the acquired swapchain image at the end of the same
    /// command buffer, so the swapchain image is only ever a copy destination.
    pub(in crate::renderer) fn present_copy_frame(&self, menu_backdrop_active: bool) -> bool {
        self.present_copy_enabled && self.offscreen_present_eligible(menu_backdrop_active)
    }

    fn offscreen_present_eligible(&self, menu_backdrop_active: bool) -> bool {
        !menu_backdrop_active
            && self.crt_scope == crate::renderer::ui::CrtScope::Off
            && !self.smaa_enabled
            && !self.egui_active
            && self.hdr_output.is_none()
            && !self.surface_diag_pending
            && !self.frame_diag_pending
            && self.config.usage.contains(wgpu::TextureUsages::COPY_DST)
    }

    /// Offscreen stand-in for the swapchain image: same size and format, so every
    /// pipeline built for the surface renders into it unchanged.
    pub(in crate::renderer) fn early_present_view(&mut self) -> wgpu::TextureView {
        let size = wgpu::Extent3d {
            width: self.config.width.max(1),
            height: self.config.height.max(1),
            depth_or_array_layers: 1,
        };
        let stale = self.early_present_target.as_ref().is_none_or(|target| {
            target.texture.size() != size || target.texture.format() != self.config.format
        });
        if stale {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("JKA early-submit frame"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.early_present_target = Some(EarlyPresentTarget { texture, view });
        }
        self.early_present_target
            .as_ref()
            .expect("early present target was just ensured")
            .view
            .clone()
    }

    /// Copy the submitted offscreen frame into the acquired swapchain image.
    pub(in crate::renderer) fn submit_early_present_copy(&self, frame: &wgpu::SurfaceTexture) {
        let target = self
            .early_present_target
            .as_ref()
            .expect("early-submit frames render into the early present target");
        let size = target.texture.size();
        if frame.texture.size() != size {
            // A resize between record and acquire; the next frame reconfigures.
            return;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("JKA early-submit present copy"),
            });
        encoder.copy_texture_to_texture(
            target.texture.as_image_copy(),
            frame.texture.as_image_copy(),
            size,
        );
        self.queue.submit([encoder.finish()]);
    }

    /// `Surface::get_current_texture()` with the JKA_SURFACE_DIAG scope and the
    /// error mapping `render()` uses for every frame.
    pub(in crate::renderer) fn acquire_surface_frame(
        &mut self,
        info: &mut FrameInfo,
    ) -> Result<wgpu::SurfaceTexture, RenderError> {
        // One-shot diagnostic for CurrentSurfaceTexture::Validation. Wgpu's
        // Validation variant means a validation error occurred inside
        // Surface::get_current_texture(). Capture that exact call in a validation
        // scope so we can print the real error rather than guessing from the
        // generic surface status. This path is completely dormant unless
        // JKA_SURFACE_DIAG=1 is set.
        let surface_scope = self
            .surface_diag_pending
            .then(|| self.device.push_error_scope(wgpu::ErrorFilter::Validation));
        if self.surface_diag_pending {
            eprintln!(
                "[JKA SURFACE DIAG] before acquire: config={}x{} format={:?} present={:?} latency={} usage={:?} alpha={:?} view_formats={:?}",
                self.config.width,
                self.config.height,
                self.config.format,
                self.config.present_mode,
                self.config.desired_maximum_frame_latency,
                self.config.usage,
                self.config.alpha_mode,
                self.config.view_formats,
            );
        }
        let acquire_started = Instant::now();
        let surface_result = self.surface.get_current_texture();
        info.cpu_acquire_ms = acquire_started.elapsed().as_secs_f64() * 1000.0;
        if let Some(scope) = surface_scope {
            let scoped_error = block_on(scope.pop());
            match scoped_error {
                Some(error) => {
                    eprintln!("[JKA SURFACE DIAG] get_current_texture validation: {error}");
                    eprintln!("[JKA SURFACE DIAG] debug: {error:?}");
                }
                None => {
                    eprintln!("[JKA SURFACE DIAG] get_current_texture scope: no validation error captured");
                }
            }
            eprintln!("[JKA SURFACE DIAG] returned: {surface_result:?}");
            self.surface_diag_pending = false;
        }
        match surface_result {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(frame),
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.flush_staged_queue_writes();
                Err(RenderError::Timeout)
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                self.flush_staged_queue_writes();
                Err(RenderError::Occluded)
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.flush_staged_queue_writes();
                Err(RenderError::Outdated)
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.flush_staged_queue_writes();
                Err(RenderError::Lost)
            }
            wgpu::CurrentSurfaceTexture::Validation => Err(RenderError::Validation),
        }
    }
}
