//! Bundled engine version catalogue.
//!
//! The Go implementation fetched the latest release families from the
//! third-party `endoflife.date` API at runtime. The rewrite drops that
//! phone-home: the supported stable major families are baked in here and
//! refreshed each KlyraDB release, which keeps the app offline-friendly and
//! free of outbound telemetry.
//!
//! Only stable majors are listed — never betas or release candidates — newest
//! first, capped at [`SUPPORTED_MAJORS`] families per engine.

use crate::engine::DbType;

/// How many stable major families KlyraDB offers per engine.
pub const SUPPORTED_MAJORS: usize = 4;

/// The bundled stable major families for `db`, newest first.
///
/// The slice is already trimmed to at most [`SUPPORTED_MAJORS`] entries; an
/// engine with fewer tracked families simply returns a shorter slice.
pub fn families(db: DbType) -> &'static [&'static str] {
	match db {
		DbType::Postgres => &["18", "17", "16", "15"],
		DbType::Mysql => &["9.4", "9.3", "8.4", "8.0"],
		DbType::Mariadb => &["11.8", "11.4", "11.2", "10.11"],
		DbType::Valkey => &["8.1", "8.0", "7.2"],
		DbType::Redis => &["7.4", "7.2", "6.2"],
		DbType::Mongodb => &["8.0", "7.0", "6.0", "5.0"],
	}
}

/// The newest stable major family for `db`, if any is tracked.
pub fn latest(db: DbType) -> Option<&'static str> {
	families(db).first().copied()
}

/// The major-version key of a full version string (the part before the first
/// dot for single-number majors, or the first two segments for dotted majors).
///
/// Used to match an installed binary's reported version against a bundled
/// family. `"17.4"` keys to `"17"`; `"8.4.2"` keys to `"8.4"` because MySQL and
/// MariaDB family identity spans two segments.
pub fn major_key(version: &str, dotted: bool) -> String {
	let segments: Vec<&str> = version.split('.').collect();
	if dotted && segments.len() >= 2 {
		format!("{}.{}", segments[0], segments[1])
	} else {
		segments.first().copied().unwrap_or(version).to_string()
	}
}

/// Whether `version` belongs to the major `family`.
pub fn major_match(version: &str, family: &str) -> bool {
	let dotted = family.contains('.');
	major_key(version, dotted) == family
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_engine_lists_stable_families_newest_first() {
		for db in DbType::all() {
			let fams = families(db);
			assert!(!fams.is_empty(), "{} has no families", db.as_str());
			assert!(fams.len() <= SUPPORTED_MAJORS);
			assert_eq!(latest(db), Some(fams[0]));
		}
	}

	#[test]
	fn major_key_handles_single_and_dotted_majors() {
		assert_eq!(major_key("17.4", false), "17");
		assert_eq!(major_key("8.4.2", true), "8.4");
		assert_eq!(major_key("18", false), "18");
	}

	#[test]
	fn major_match_respects_family_shape() {
		assert!(major_match("17.4", "17"));
		assert!(major_match("8.4.2", "8.4"));
		assert!(!major_match("8.0.1", "8.4"));
		assert!(!major_match("16.2", "17"));
	}
}
