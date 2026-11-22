//! ribozym — API-first core (P1, §24 rule 5).
//!
//! Everything the CLI does is a thin wrapper over these public items.
//! Language policy: this file is English (§24 rule 1).

pub mod agent;
pub mod agents;
pub mod archive;
pub mod cli;
pub mod clock;
pub mod config;
pub mod error;
pub mod events;
pub mod fsm;
pub mod gates;
pub mod gateway;
pub mod hash;
pub mod inference;
pub mod ledger;
pub mod lock;
pub mod logging;
pub mod models;
pub mod orchestrator;
pub mod pipeline;
pub mod repair;
pub mod scheduler;
pub mod select;
pub mod state;
pub mod surface;
pub mod validate;
pub mod web;

pub use error::{Error, Result};
