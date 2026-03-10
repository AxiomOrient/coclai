//! Public facade for the coclai workspace.
//! Default path: use this crate first. Use `coclai::runtime` for low-level control.

mod adapters;
mod appserver;
pub mod automation;
mod domain;
mod ergonomic;
pub mod plugin;
pub mod runtime;
#[cfg(test)]
pub(crate) mod test_fixtures;

pub use adapters::web;
pub use appserver::{methods as rpc_methods, AppServer};
pub use domain::artifact;
pub use ergonomic::{quick_run, quick_run_with_profile, QuickRunError, Workflow, WorkflowConfig};
pub use plugin::{FilteredPostHook, FilteredPreHook, HookMatcher};
pub use runtime::ShellCommandHook;
