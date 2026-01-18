//! Test-only adapters that live inside the domain crate for convenience.
//!
//! These are intended purely for unit testing and local demos. Real adapters
//! (DynamoDB, Firestore, etc.) will live in separate crates.

pub mod memory_repo;
