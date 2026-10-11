//! One persistent finisher, one outstanding job, same-frame ordered submission.
use crate::renderer::{PlanarReflectionDebugMode, PlanarReflectionMode, Renderer};
use std::{sync::mpsc, thread, time::Instant};

struct Job {
    encoder: wgpu::CommandEncoder,
    measure: bool,
}

pub(in crate::renderer) struct Finished {
    pub buffer: wgpu::CommandBuffer,
    pub milliseconds: f64,
}

pub(in crate::renderer) struct EncoderFinisher {
    request: Option<mpsc::SyncSender<Job>>,
    response: Option<mpsc::Receiver<Finished>>,
    join: Option<thread::JoinHandle<()>>,
    pending: bool,
}

impl EncoderFinisher {
    pub fn new() -> Result<Self, String> {
        let (request, requests) = mpsc::sync_channel::<Job>(1);
        let (results, response) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("jka-encoder-finish".into())
            .spawn(move || {
                while let Ok(job) = requests.recv() {
                    let started = job.measure.then(Instant::now);
                    let buffer = job.encoder.finish();
                    let milliseconds = started.map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0);
                    if results
                        .send(Finished {
                            buffer,
                            milliseconds,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            request: Some(request),
            response: Some(response),
            join: Some(join),
            pending: false,
        })
    }

    pub fn dispatch(&mut self, encoder: wgpu::CommandEncoder, measure: bool) -> Result<(), String> {
        if self.pending {
            return Err("encoder finish already pending".into());
        }
        self.request
            .as_ref()
            .ok_or("encoder finish worker stopped")?
            .send(Job { encoder, measure })
            .map_err(|_| "encoder finish worker disconnected".to_string())?;
        self.pending = true;
        Ok(())
    }

    pub fn wait(&mut self) -> Result<Finished, String> {
        if !self.pending {
            return Err("no encoder finish pending".into());
        }
        let result = self
            .response
            .as_ref()
            .ok_or("encoder finish worker stopped")?
            .recv()
            .map_err(|_| "encoder finish worker failed".to_string());
        self.pending = false;
        result
    }
}

impl Drop for EncoderFinisher {
    fn drop(&mut self) {
        // Closing the result receiver first also releases an unconsumed result
        // and prevents shutdown from blocking on the bounded result channel.
        self.response.take();
        self.request.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub(super) fn eligible(
    mode: u32,
    samples: u32,
    ocean: bool,
    optical: bool,
    rt: bool,
    reflection: PlanarReflectionMode,
    debug: PlanarReflectionDebugMode,
    gpu_driven: bool,
) -> bool {
    mode != 0
        && samples == 1
        && !ocean
        && !optical
        && !rt
        && !gpu_driven
        && reflection == PlanarReflectionMode::Off
        && debug == PlanarReflectionDebugMode::Off
}

pub(super) fn ordered_buffers(
    prefix: Option<wgpu::CommandBuffer>,
    tail: wgpu::CommandBuffer,
    readback: Option<wgpu::CommandBuffer>,
) -> impl Iterator<Item = wgpu::CommandBuffer> {
    prefix
        .into_iter()
        .chain(std::iter::once(tail))
        .chain(readback)
}

impl Renderer {
    pub(in crate::renderer) fn set_encoder_finish_threads(
        &mut self,
        mode: u32,
    ) -> Result<(), String> {
        if mode > 2 {
            return Err("expected 0|1|2".into());
        }
        if self.encoder_finish_threads == mode {
            return Ok(());
        }
        let worker = if mode == 2 {
            Some(EncoderFinisher::new()?)
        } else {
            None
        };
        self.encoder_finisher = worker;
        self.encoder_finish_threads = mode;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_finish_requires_the_conservative_single_sample_raster_path() {
        let off = PlanarReflectionMode::Off;
        let debug = PlanarReflectionDebugMode::Off;
        assert!(!eligible(0, 1, false, false, false, off, debug, false));
        assert!(eligible(1, 1, false, false, false, off, debug, false));
        assert!(eligible(2, 1, false, false, false, off, debug, false));
        assert!(!eligible(2, 4, false, false, false, off, debug, false));
        assert!(!eligible(2, 1, true, false, false, off, debug, false));
        assert!(!eligible(2, 1, false, true, false, off, debug, false));
        assert!(!eligible(2, 1, false, false, true, off, debug, false));
        assert!(!eligible(2, 1, false, false, false, off, debug, true));
        assert!(!eligible(
            2,
            1,
            false,
            false,
            false,
            PlanarReflectionMode::Environment,
            debug,
            false
        ));
        assert!(!eligible(
            2,
            1,
            false,
            false,
            false,
            off,
            PlanarReflectionDebugMode::BindingTest,
            false
        ));
    }

    #[test]
    fn idle_worker_shutdown_and_empty_wait_are_bounded() {
        let mut worker = EncoderFinisher::new().unwrap();
        assert!(worker.wait().is_err());
        drop(worker);
    }
}

#[cfg(test)]
#[path = "finish_tests.rs"]
mod gpu_tests;
