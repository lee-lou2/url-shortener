//! API module.
//!
//! Contains HTTP handlers, routes, schemas, and middleware.

pub mod handlers;
pub mod middlewares;
pub mod routes;
pub mod schemas;
pub mod state;

// These types are used in integration tests
#[allow(unused_imports)]
pub use handlers::{HealthResponse, ReadinessResponse};
pub use routes::create_routes;
pub use state::AppState;
