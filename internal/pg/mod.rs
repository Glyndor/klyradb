//! PostgreSQL engine.
//!
//! Postgres ships one binary tree per major version (`.../<major>/bin`), so this
//! engine discovers installs by scanning the conventional locations for a
//! `pg_ctl`. Instances are initialized with `initdb` (trust auth on loopback)
//! and driven through `pg_ctl`; the server keeps its pid in
//! `<data_dir>/postmaster.pid`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::engine::paths::{engines_dir, snap_dir};
use crate::engine::proc::{check_pid, port_open};
use crate::engine::{DbType, Engine, EngineError, Instance, Status, Version};
use crate::versions;

/// How long to wait for the server to begin accepting connections.
const START_TIMEOUT: Duration = Duration::from_secs(15);

/// The PostgreSQL engine.
pub struct PgEngine;

impl PgEngine {
	/// Builds the PostgreSQL engine.
	pub fn new() -> PgEngine {
		PgEngine
	}

	fn bin_for(&self, major: &str) -> Result<PathBuf, EngineError> {
		installed_bins().remove(major).ok_or_else(|| {
			EngineError::NotInstalled(format!("PostgreSQL {major} is not installed"))
		})
	}
}

impl Default for PgEngine {
	fn default() -> Self {
		PgEngine::new()
	}
}

impl Engine for PgEngine {
	fn db_type(&self) -> DbType {
		DbType::Postgres
	}

	fn versions(&self) -> Vec<Version> {
		let found = installed_bins();
		versions::families(DbType::Postgres)
			.iter()
			.map(|&major| {
				let bin = found.get(major);
				Version {
					db_type: DbType::Postgres,
					major: major.to_string(),
					bin_path: bin
						.map(|p| p.to_string_lossy().into_owned())
						.unwrap_or_default(),
					installed: bin.is_some(),
					label: format!("PostgreSQL {major}"),
					latest_patch: major.to_string(),
					installed_version: bin
						.map(|p| detect_version(p).unwrap_or_default())
						.unwrap_or_default(),
				}
			})
			.collect()
	}

	fn create(&self, inst: &mut Instance) -> Result<(), EngineError> {
		let bin = self.bin_for(&inst.version)?;
		if let Some(parent) = Path::new(&inst.log_file).parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::create_dir_all(&inst.data_dir)?;
		restrict_dir(&inst.data_dir)?;
		run(
			bin.join("initdb"),
			&[
				"-D",
				&inst.data_dir,
				"-U",
				&inst.user,
				"--auth=trust",
				"--encoding=UTF8",
			],
			"initdb",
		)
	}

	fn start(&self, inst: &Instance) -> Result<(), EngineError> {
		let bin = self.bin_for(&inst.version)?;
		run(
			bin.join("pg_ctl"),
			&[
				"-D",
				&inst.data_dir,
				"-l",
				&inst.log_file,
				"-o",
				&format!("-p {} -k {}", inst.port, inst.data_dir),
				"start",
			],
			"pg_ctl start",
		)?;
		let deadline = Instant::now() + START_TIMEOUT;
		while Instant::now() < deadline {
			if port_open(inst.port) {
				return Ok(());
			}
			sleep(Duration::from_millis(200));
		}
		Err(EngineError::Tooling(format!(
			"postgres did not open port {} within {}s",
			inst.port,
			START_TIMEOUT.as_secs()
		)))
	}

	fn stop(&self, inst: &Instance) -> Result<(), EngineError> {
		let bin = self.bin_for(&inst.version)?;
		run(
			bin.join("pg_ctl"),
			&["-D", &inst.data_dir, "-m", "fast", "stop"],
			"pg_ctl stop",
		)
	}

	fn delete(&self, inst: &Instance) -> Result<(), EngineError> {
		let _ = self.stop(inst);
		let _ = std::fs::remove_dir_all(&inst.data_dir);
		let _ = std::fs::remove_file(&inst.log_file);
		Ok(())
	}

	fn check_status(&self, inst: &Instance) -> Status {
		let pid_file = Path::new(&inst.data_dir).join("postmaster.pid");
		if check_pid(&pid_file).is_some() || port_open(inst.port) {
			Status::Running
		} else {
			Status::Stopped
		}
	}
}

/// Discovers installed Postgres major versions, mapping each major to its
/// `bin` directory. Scans the snap bundle when confined, otherwise the user
/// engines directory followed by the system trees.
fn installed_bins() -> BTreeMap<String, PathBuf> {
	let bases: Vec<PathBuf> = if let Some(snap) = snap_dir() {
		vec![snap.join("usr/lib/postgresql")]
	} else {
		vec![
			engines_dir().join("pg"),
			PathBuf::from("/usr/lib/postgresql"),
			PathBuf::from("/opt/postgresql"),
		]
	};

	let mut found = BTreeMap::new();
	for base in bases {
		let Ok(entries) = std::fs::read_dir(&base) else {
			continue;
		};
		for entry in entries.flatten() {
			if !entry.path().is_dir() {
				continue;
			}
			let bin = entry.path().join("bin");
			if bin.join("pg_ctl").is_file() {
				if let Some(name) = entry.file_name().to_str() {
					found.entry(name.to_string()).or_insert(bin);
				}
			}
		}
	}
	found
}

/// Reads the major.minor reported by `<bin>/postgres --version` (last field).
fn detect_version(bin: &Path) -> Option<String> {
	let out = std::process::Command::new(bin.join("postgres"))
		.arg("--version")
		.output()
		.ok()?;
	String::from_utf8_lossy(&out.stdout)
		.split_whitespace()
		.next_back()
		.map(str::to_string)
}

/// Runs an engine command, turning a non-zero exit into a tooling error that
/// carries the combined output.
fn run(program: PathBuf, args: &[&str], what: &str) -> Result<(), EngineError> {
	let out = std::process::Command::new(&program).args(args).output()?;
	if out.status.success() {
		return Ok(());
	}
	let mut msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
	if msg.is_empty() {
		msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
	}
	Err(EngineError::Tooling(format!("{what}: {msg}")))
}

#[cfg(unix)]
fn restrict_dir(path: &str) -> Result<(), EngineError> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
	Ok(())
}

#[cfg(not(unix))]
fn restrict_dir(_path: &str) -> Result<(), EngineError> {
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reports_postgres_type() {
		assert_eq!(PgEngine::new().db_type(), DbType::Postgres);
	}

	#[test]
	fn versions_list_covers_the_bundled_families_newest_first() {
		let listed = PgEngine::new().versions();
		let fams = versions::families(DbType::Postgres);
		assert_eq!(listed.len(), fams.len());
		assert_eq!(listed[0].major, fams[0]);
		assert_eq!(listed[0].label, format!("PostgreSQL {}", fams[0]));
	}

	#[test]
	fn bin_for_an_absent_major_is_not_installed_error() {
		// "99" is never a real installed major.
		let err = PgEngine::new().bin_for("99").unwrap_err();
		assert!(matches!(err, EngineError::NotInstalled(_)));
	}
}
