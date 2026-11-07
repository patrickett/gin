//! Executable design scenarios, deliberately outside the parser's public API.

#[path = "borrow_model/mod.rs"]
mod model;
#[path = "borrow_model/multi_place_traces.rs"]
mod multi_place_traces;
#[path = "borrow_model/ownership_call_traces.rs"]
mod ownership_call_traces;
#[path = "borrow_model/ownership_calls.rs"]
mod ownership_calls;
#[path = "borrow_model/ownership_limits.rs"]
mod ownership_limits;
#[path = "borrow_model/ownership_scenarios.rs"]
mod ownership_scenarios;
#[path = "borrow_model/ownership_support.rs"]
mod ownership_support;
#[path = "borrow_model/ownership_traces.rs"]
mod ownership_traces;
#[path = "borrow_model/region_requirements.rs"]
mod region_requirements;
#[path = "borrow_model/scenarios.rs"]
mod scenarios;
#[path = "borrow_model/source.rs"]
mod source;
#[path = "borrow_model/support_traces.rs"]
mod support_traces;
#[path = "borrow_model/traces.rs"]
mod traces;
