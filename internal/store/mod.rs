//! Persistence for the managed-instance registry.
//!
//! The whole registry lives in a single `instances.json` file under the base
//! directory. Writes are atomic (write-then-rename) so an interrupted save can
//! never leave a half-written, unparseable registry behind, and the file is
//! created with owner-only permissions because it describes the user's local
//! services.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::engine::Instance;

/// Reads and writes the instance registry at `<base>/instances.json`.
pub struct Store {
	path: PathBuf,
}

/// A failure reading or writing the registry.
#[derive(Debug)]
pub enum StoreError {
	/// A filesystem operation failed.
	Io(String),
	/// The on-disk registry was not valid JSON.
	Corrupt(String),
}

impl std::fmt::Display for StoreError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			StoreError::Io(m) => write!(f, "{m}"),
			StoreError::Corrupt(m) => write!(f, "instances.json is corrupt: {m}"),
		}
	}
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
	fn from(e: std::io::Error) -> Self {
		StoreError::Io(e.to_string())
	}
}

impl Store {
	/// Opens the registry under `base_dir`, creating the directory (owner-only)
	/// if it does not yet exist. Does not touch the registry file itself.
	pub fn new(base_dir: impl AsRef<Path>) -> Result<Store, StoreError> {
		let base = base_dir.as_ref();
		fs::create_dir_all(base)?;
		restrict_dir(base)?;
		Ok(Store {
			path: base.join("instances.json"),
		})
	}

	/// The path to the backing file.
	pub fn path(&self) -> &Path {
		&self.path
	}

	/// Loads every persisted instance. A missing registry is an empty registry,
	/// not an error, so first launch needs no special-casing.
	pub fn read(&self) -> Result<Vec<Instance>, StoreError> {
		let bytes = match fs::read(&self.path) {
			Ok(b) => b,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
			Err(e) => return Err(e.into()),
		};
		if bytes.iter().all(u8::is_ascii_whitespace) {
			return Ok(Vec::new());
		}
		serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt(e.to_string()))
	}

	/// Persists the registry atomically: the new contents are written to a
	/// sibling temporary file, flushed, restricted to owner-only and then
	/// renamed over the target, so a reader never observes a partial write.
	pub fn write(&self, instances: &[Instance]) -> Result<(), StoreError> {
		let json =
			serde_json::to_vec_pretty(instances).map_err(|e| StoreError::Corrupt(e.to_string()))?;
		let tmp = self.path.with_extension("json.tmp");

		let mut f = fs::File::create(&tmp)?;
		restrict_file(&f)?;
		f.write_all(&json)?;
		f.sync_all()?;
		drop(f);

		fs::rename(&tmp, &self.path)?;
		Ok(())
	}
}

#[cfg(unix)]
fn restrict_dir(path: &Path) -> Result<(), StoreError> {
	use std::os::unix::fs::PermissionsExt;
	fs::set_permissions(path, fs::Permissions::from_mode(0o750))?;
	Ok(())
}

#[cfg(unix)]
fn restrict_file(file: &fs::File) -> Result<(), StoreError> {
	use std::os::unix::fs::PermissionsExt;
	file.set_permissions(fs::Permissions::from_mode(0o600))?;
	Ok(())
}

#[cfg(not(unix))]
fn restrict_dir(_path: &Path) -> Result<(), StoreError> {
	Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_file: &fs::File) -> Result<(), StoreError> {
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::engine::{DbType, Status};

	fn sample(id: &str) -> Instance {
		Instance {
			id: id.into(),
			name: "dev".into(),
			db_type: DbType::Postgres,
			version: "17".into(),
			port: 5432,
			data_dir: "/d".into(),
			log_file: "/l".into(),
			pid_file: "/p".into(),
			conf_file: String::new(),
			user: "postgres".into(),
			status: Status::Stopped,
			created_at: "2026-06-29T00:00:00Z".into(),
			last_error: String::new(),
			upgrade_version: String::new(),
			patch_update: String::new(),
		}
	}

	#[test]
	fn missing_registry_reads_as_empty() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::new(dir.path()).unwrap();
		assert!(store.read().unwrap().is_empty());
	}

	#[test]
	fn write_then_read_round_trips() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::new(dir.path()).unwrap();
		let want = vec![sample("aaa111"), sample("bbb222")];
		store.write(&want).unwrap();
		assert_eq!(store.read().unwrap(), want);
	}

	#[test]
	fn write_leaves_no_temp_file_behind() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::new(dir.path()).unwrap();
		store.write(&[sample("aaa111")]).unwrap();
		let leftovers: Vec<_> = fs::read_dir(dir.path())
			.unwrap()
			.filter_map(Result::ok)
			.filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
			.collect();
		assert!(leftovers.is_empty(), "temp file was not renamed away");
	}

	#[test]
	fn corrupt_registry_is_reported_not_silently_dropped() {
		let dir = tempfile::tempdir().unwrap();
		let store = Store::new(dir.path()).unwrap();
		fs::write(store.path(), b"{not json").unwrap();
		assert!(matches!(store.read(), Err(StoreError::Corrupt(_))));
	}

	#[cfg(unix)]
	#[test]
	fn registry_file_is_owner_only() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().unwrap();
		let store = Store::new(dir.path()).unwrap();
		store.write(&[sample("aaa111")]).unwrap();
		let mode = fs::metadata(store.path()).unwrap().permissions().mode();
		assert_eq!(mode & 0o777, 0o600);
	}
}
