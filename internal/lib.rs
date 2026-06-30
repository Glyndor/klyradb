//! KlyraDB core library.
//!
//! Holds the engine-agnostic domain model and the orchestration logic that the
//! Tauri bridge binds to the webview. The crate is split by concern, one module
//! per folder, mirroring the layout the Go implementation used before the
//! Rust/Tauri rewrite.

pub mod engine;
pub mod i18n;
pub mod install;
pub mod manager;
pub mod mariadb;
pub mod mongodb;
pub mod mysql;
pub mod pg;
pub mod redis;
pub mod store;
pub mod versions;
