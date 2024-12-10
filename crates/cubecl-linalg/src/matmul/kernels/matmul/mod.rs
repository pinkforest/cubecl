mod base;
mod config;

mod algorithm;

pub use algorithm::{standard, Algorithm};
pub use base::{launch, launch_ref, matmul_select_kernel};
pub use config::{create_stage_dim, AdvancedConfig};
