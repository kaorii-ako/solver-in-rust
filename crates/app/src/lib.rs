//! Everything the GUI, TUI and CLI share: case files, geometry preparation,
//! the run thread, results, history, comparison and reports. The front ends
//! only send commands and read events, so they always agree.

pub mod case;
pub mod compare;
pub mod image;
pub mod prepare;
pub mod report;
pub mod result;
pub mod run;
pub mod time;

pub use case::{Case, Device, GroundMode, Nose, PartSpec, Quality, UpAxis};
pub use cfd_lbm_gpu::probe as gpu_name;
pub use prepare::{load_geometry, plan_domain, DomainPlan, GeometryInfo};
pub use result::{list_runs, RunResult};
pub use run::{start, Event, Progress, RunHandle, RunOutput, Slice};
