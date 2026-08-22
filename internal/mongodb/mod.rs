//! MongoDB engine.
//!
//! `mongod` is started forked from a generated YAML config bound to loopback;
//! shutdown prefers an admin `shutdown` command issued through `mongosh` (or the
//! legacy `mongo` shell) and falls back to signalling the pidfile.

use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::engine::paths::find_binary;
use crate::engine::proc::{check_pid, port_open, signal_int};
use crate::engine::{DbType, Engine, EngineError, Instance, Status, Version};
use crate::versions;

/// How long to wait for `mongod` to begin accepting connections.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// The MongoDB engine.
pub struct MongoEngine;

impl MongoEngine {
	/// Builds the MongoDB engine.
	pub fn new() -> MongoEngine {
		MongoEngine
	}

	fn detect_version(&self) -> Option<String> {
		let bin = find_binary("mongodb", "mongod")?;
		let out = std::process::Command::new(&bin)
			.arg("--version")
			.output()
			.ok()?;
		parse_version(&String::from_utf8_lossy(&out.stdout))
	}
}

impl Default for MongoEngine {
	fn default() -> Self {
		MongoEngine::new()
	}
}

impl Engine for MongoEngine {
	fn db_type(&self) -> DbType {
		DbType::Mongodb
	}

	fn versions(&self) -> Vec<Version> {
		let installed = self.detect_version();
		let bin_dir = find_binary("mongodb", "mongod")
			.and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
			.unwrap_or_default();
		versions::families(DbType::Mongodb)
			.iter()
			.map(|&major| {
				let is_installed = installed
					.as_deref()
					.is_some_and(|v| versions::major_match(v, major));
				Version {
					db_type: DbType::Mongodb,
					major: major.to_string(),
					bin_path: if is_installed {
						bin_dir.clone()
					} else {
						String::new()
					},
					installed: is_installed,
					label: format!("MongoDB {major}"),
					latest_patch: major.to_string(),
					installed_version: if is_installed {
						installed.clone().unwrap_or_default()
					} else {
						String::new()
					},
				}
			})
			.collect()
	}

	fn create(&self, inst: &mut Instance) -> Result<(), EngineError> {
		if find_binary("mongodb", "mongod").is_none() {
			return Err(EngineError::NotInstalled(
				"mongod not found — install it to create this instance".into(),
			));
		}
		for dir in [
			inst.data_dir.as_str(),
			parent_of(&inst.log_file),
			parent_of(&inst.pid_file),
			parent_of(&inst.conf_file),
		] {
			if !dir.is_empty() {
				std::fs::create_dir_all(dir)?;
			}
		}
		restrict_dir(&inst.data_dir)?;
		std::fs::write(&inst.conf_file, render_conf(inst))?;
		restrict_file(&inst.conf_file)?;
		Ok(())
	}

	fn start(&self, inst: &Instance) -> Result<(), EngineError> {
		let bin = find_binary("mongodb", "mongod")
			.ok_or_else(|| EngineError::NotInstalled("mongod".into()))?;
		let out = std::process::Command::new(&bin)
			.args(["--config", &inst.conf_file])
			.output()?;
		if !out.status.success() {
			return Err(EngineError::Tooling(format!(
				"mongod failed to start: {}",
				String::from_utf8_lossy(&out.stderr).trim()
			)));
		}
		let deadline = Instant::now() + START_TIMEOUT;
		while Instant::now() < deadline {
			sleep(Duration::from_millis(300));
			if port_open(inst.port) {
				return Ok(());
			}
		}
		Err(EngineError::Tooling(format!(
			"mongod did not open port {} within {}s",
			inst.port,
			START_TIMEOUT.as_secs()
		)))
	}

	fn stop(&self, inst: &Instance) -> Result<(), EngineError> {
		for shell in ["mongosh", "mongo"] {
			if let Some(bin) = find_binary("mongodb", shell) {
				let ok = std::process::Command::new(&bin)
					.args([
						"--port",
						&inst.port.to_string(),
						"--eval",
						"db.adminCommand({shutdown:1})",
						"admin",
					])
					.status()
					.map(|s| s.success())
					.unwrap_or(false);
				if ok {
					return Ok(());
				}
			}
		}
		if signal_int(Path::new(&inst.pid_file)) {
			Ok(())
		} else {
			Err(EngineError::Tooling("failed to signal mongod".into()))
		}
	}

	fn delete(&self, inst: &Instance) -> Result<(), EngineError> {
		let _ = self.stop(inst);
		let _ = std::fs::remove_dir_all(&inst.data_dir);
		let _ = std::fs::remove_file(&inst.log_file);
		let _ = std::fs::remove_file(&inst.pid_file);
		let _ = std::fs::remove_file(&inst.conf_file);
		Ok(())
	}

	fn check_status(&self, inst: &Instance) -> Status {
		if check_pid(Path::new(&inst.pid_file)).is_some() || port_open(inst.port) {
			Status::Running
		} else {
			Status::Stopped
		}
	}
}

/// Renders the forked-server YAML config for an instance.
fn render_conf(inst: &Instance) -> String {
	format!(
		"storage:\n  dbPath: {}\nsystemLog:\n  destination: file\n  path: {}\n  logAppend: true\nnet:\n  port: {}\n  bindIp: 127.0.0.1\nprocessManagement:\n  fork: true\n  pidFilePath: {}\n",
		inst.data_dir, inst.log_file, inst.port, inst.pid_file
	)
}

/// Parses a `mongod --version` block, whose first line is `db version vX.Y.Z`.
fn parse_version(out: &str) -> Option<String> {
	out.lines()
		.map(str::trim)
		.find_map(|l| l.strip_prefix("db version v"))
		.map(str::to_string)
}

fn parent_of(path: &str) -> &str {
	Path::new(path)
		.parent()
		.and_then(Path::to_str)
		.unwrap_or("")
}

#[cfg(unix)]
fn restrict_dir(path: &str) -> Result<(), EngineError> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
	Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &str) -> Result<(), EngineError> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
	Ok(())
}

#[cfg(not(unix))]
fn restrict_dir(_path: &str) -> Result<(), EngineError> {
	Ok(())
}

#[cfg(not(unix))]
fn restrict_file(_path: &str) -> Result<(), EngineError> {
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sample() -> Instance {
		Instance {
			id: "m00001".into(),
			name: "docs".into(),
			db_type: DbType::Mongodb,
			version: "8.0".into(),
			port: 27017,
			data_dir: "/base/data/m00001".into(),
			log_file: "/base/logs/m00001.log".into(),
			pid_file: "/base/pids/m00001.pid".into(),
			conf_file: "/base/conf/m00001.yml".into(),
			user: String::new(),
			status: Status::Stopped,
			created_at: "2026-06-29T00:00:00Z".into(),
			last_error: String::new(),
			upgrade_version: String::new(),
			patch_update: String::new(),
		}
	}

	#[test]
	fn reports_mongodb_type() {
		assert_eq!(MongoEngine::new().db_type(), DbType::Mongodb);
	}

	#[test]
	fn conf_forks_binds_loopback_and_points_at_instance_paths() {
		let conf = render_conf(&sample());
		assert!(conf.contains("dbPath: /base/data/m00001"));
		assert!(conf.contains("port: 27017"));
		assert!(conf.contains("bindIp: 127.0.0.1"));
		assert!(conf.contains("fork: true"));
		assert!(conf.contains("pidFilePath: /base/pids/m00001.pid"));
	}

	#[test]
	fn version_is_parsed_from_db_version_line() {
		let out = "db version v8.0.5\nBuild Info: ...";
		assert_eq!(parse_version(out), Some("8.0.5".to_string()));
		assert_eq!(parse_version("no version"), None);
	}

	#[test]
	fn versions_list_covers_the_bundled_families() {
		let listed = MongoEngine::new().versions();
		assert_eq!(listed.len(), versions::families(DbType::Mongodb).len());
		assert_eq!(listed[0].label, format!("MongoDB {}", listed[0].major));
	}
}
