//! Event system for agent-user interaction
//!
//! This module provides an event-driven architecture for communication
//! between the agent and UI components.

pub mod bus;
pub mod types;

pub use bus::EventBus;
pub use types::*;
