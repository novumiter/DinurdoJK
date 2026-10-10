//! Conservative staggered-grid shallow-water solver for coastal simulation.
//!
//! Solver quantities use SI units: metres, seconds, metres/second, and
//! metres/second squared. JKA bathymetry therefore converts world units using
//! 40 world units per metre before constructing a [`Domain`]. The core has no
//! renderer, WGPU, game-state, or threading dependencies.
//!
//! The pressure, donor-flux, shared positivity-limiter, and divergence order
//! are adapted from Saltreach (MIT, Copyright 2026 Techartist). See
//! `LICENSE-Saltreach-MIT` and `docs/coastal-checkpoint2.md`.

mod solver;
mod surface;
pub mod validation;

pub use solver::{
    AdvanceReport, BoundaryTarget, Diagnostics, Domain, FrameStepper, Grid, Solver, SolverOptions,
    StepFields, StepReport,
};
pub use surface::{reconstruct_surface, SurfaceSample, FILM_DEPTH_METRES};
