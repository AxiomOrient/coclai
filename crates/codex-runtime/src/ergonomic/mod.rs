mod config;
mod oneshot;
mod paths;
mod workflow;

pub use config::WorkflowConfig;
pub use oneshot::{quick_run, quick_run_with_profile, QuickRunError};
pub use workflow::Workflow;

#[cfg(test)]
pub(crate) use oneshot::fold_quick_run;

#[cfg(test)]
mod tests;
