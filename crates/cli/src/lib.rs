pub mod client;
pub mod manage;
pub mod session;
pub mod ssh;
pub mod validate;

/// Generated gRPC types for the anvil API.
pub mod api {
    tonic::include_proto!("anvil");
}
