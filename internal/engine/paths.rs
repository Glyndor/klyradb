//! Filesystem locations KlyraDB uses, snap-aware.
//!
//! Inside a snap the app is confined, so engine binaries are only ever the
//! bundled ones under `$SNAP` and user data lives in `$SNAP_USER_COMMON` (which
//! survives refreshes but is purged on removal). Outside a snap, data lives
//! under the XDG data dir and the user may drop custom engine builds into the
//! engines directory, which is searched ahead of the system paths.

use std::path::PathBuf;

/// The `$SNAP` root when running inside a snap, otherwise `None`.
pub fn snap_dir() -> Option<PathBuf> {
	std::env::var_os("SNAP")
		.map(PathBuf::from)
		.filter(|p| !p.as_os_str().is_empty())
}

/// The base data directory: `$SNAP_USER_COMMON` inside a snap, else
/// `$XDG_DATA_HOME/klyradb` (falling back to `~/.local/share/klyradb`).
pub fn base_dir() -> PathBuf {
	if let Some(common) = std::env::var_os("SNAP_USER_COMMON") {
		if !common.is_empty() {
			return PathBuf::from(common);
		}
	}
	if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
		if !xdg.is_empty() {
			return PathBuf::from(xdg).join("klyradb");
		}
	}
	let home = std::env::var_os("HOME")
		.map(PathBuf::from)
		.unwrap_or_default();
	home.join(".local").join("share").join("klyradb")
}

/// The directory where users may drop custom engine builds, searched before the
/// system paths. Only meaningful outside a snap.
pub fn engines_dir() -> PathBuf {
	base_dir().join("engines")
}

/// Resolves an engine executable by name, honouring snap confinement.
///
/// Inside a snap only the bundled `$SNAP` binaries are considered — the host
/// system is never used. Outside a snap the user's engines directory wins, then
/// the conventional system locations, then `$PATH`. Returns `None` when nothing
/// is found.
pub fn find_binary(engine_subdir: &str, name: &str) -> Option<PathBuf> {
	if let Some(snap) = snap_dir() {
		for rel in ["usr/bin", "usr/local/bin"] {
			let p = snap.join(rel).join(name);
			if p.is_file() {
				return Some(p);
			}
		}
		return None;
	}

	let candidates = [
		engines_dir().join(engine_subdir).join("bin").join(name),
		PathBuf::from("/usr/bin").join(name),
		PathBuf::from("/usr/local/bin").join(name),
	];
	for p in candidates {
		if p.is_file() {
			return Some(p);
		}
	}
	which(name)
}

/// Searches `$PATH` for an executable, like `command -v`.
fn which(name: &str) -> Option<PathBuf> {
	let path = std::env::var_os("PATH")?;
	std::env::split_paths(&path)
		.map(|dir| dir.join(name))
		.find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn engines_dir_is_under_base_dir() {
		assert!(engines_dir().starts_with(base_dir()));
		assert!(engines_dir().ends_with("engines"));
	}

	#[test]
	fn which_finds_an_absolute_match_on_path() {
		// `sh` exists on every unix host the tests run on.
		if cfg!(unix) {
			let found = which("sh");
			assert!(found.is_some(), "sh should be resolvable on PATH");
			assert!(found.unwrap().is_absolute());
		}
	}

	#[test]
	fn which_returns_none_for_a_nonexistent_binary() {
		assert!(which("klyradb-definitely-not-a-real-binary-xyz").is_none());
	}
}
