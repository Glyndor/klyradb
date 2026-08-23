#![no_main]
//! Fuzzes the version-string matching against arbitrary input.
//!
//! Version strings come from each engine's `--version` output, which is outside
//! KlyraDB's control, so the family-matching helpers must handle any bytes
//! without panicking.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	if let Ok(s) = std::str::from_utf8(data) {
		let _ = klyradb::versions::major_key(s, true);
		let _ = klyradb::versions::major_key(s, false);
		let _ = klyradb::versions::major_match(s, "8.4");
	}
});
