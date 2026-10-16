//! Host checks and setup, GPU listing, eligibility and VFIO reservation, Reims vGPU host checks
//! (port of the Go app's internal/core host.go, gpu.go and reimshost.go). See API.md.

mod compat;
mod gpu;
mod host;
mod reims;
#[cfg(test)]
mod testutil;

pub use gpu::{Compat, Gpu, PciFunc, is_kepler};
pub use host::{
    Check, CmdError, ExtraKextsFn, Host, HostError, LookPathFn, OutputFn, ReimsQemuFn, RunFn,
    Status,
};
pub use reims::{ReimsReady, reims_map_count};
