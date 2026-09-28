pub mod model;
pub mod provider;
pub mod task_control;
pub mod tasks;
// Shared production host-effect implementation; no Python or desktop runtime dependency.
#[path = "../../mission-control/src-tauri/src/execution.rs"]
pub mod execution;
pub mod ui;
pub mod worker;

pub mod activity;
pub mod commands;
pub mod conversations;

pub mod review;

pub mod doctor;

pub mod dependencies;

pub mod branch;

pub mod planner;

pub mod assignment;

pub mod planning_context;

pub mod dispatch;

pub mod task_view;

pub mod mission_work;

pub mod metrics;

pub mod selection;

pub mod missions;

pub mod understanding;

pub mod client_timing;

pub mod workstation;

pub mod wayfinder;

pub mod reading;

pub mod run_boundary;

pub mod assessment;

pub mod agent;

pub mod architecture;

pub mod command_intent;

pub mod console_command;

pub mod planner_command;

pub mod control_command;

pub mod selection_command;
pub mod selection_store;

pub mod inference_admission;

pub mod inference_runtime;

pub mod inference_profile;

pub mod qualification_runner;

pub mod qualification;

pub mod qualification_oracle;
