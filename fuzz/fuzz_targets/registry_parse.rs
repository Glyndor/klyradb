#![no_main]
//! Fuzzes the instance-registry parser against arbitrary bytes.
//!
//! `instances.json` is the one persisted file the app reads back at startup, so
//! a malformed or hostile registry must be rejected, never panic the process.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	let _ = serde_json::from_slice::<Vec<klyradb::engine::Instance>>(data);
});
