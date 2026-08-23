//! Redis and Valkey engines.
//!
//! Valkey is the BSD-licensed fork of Redis and is wire- and config-compatible,
//! so both are driven by one [`RespEngine`] parameterised by the binary names
//! and engine identity. The server is started daemonized from a generated
//! config file; shutdown prefers the CLI's `shutdown nosave` and falls back to
//! signalling the pidfile.

use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::engine::paths::find_binary;
use crate::engine::proc::{check_pid, port_open, signal_int};
use crate::engine::{DbType, Engine, EngineError, Instance, Status, Version};
use crate::versions;

/// How long to wait for the server to begin accepting connections.
const START_TIMEOUT: Duration = Duration::from_secs(15);

/// A Redis-protocol engine (Redis or Valkey).
pub struct RespEngine {
	db: DbType,
	server_bin: &'static str,
	cli_bin: &'static str,
	subdir: &'static str,
	label: &'static str,
}

impl RespEngine {
	/// The Redis engine.
	pub fn redis() -> RespEngine {
		RespEngine {
			db: DbType::Redis,
			server_bin: "redis-server",
			cli_bin: "redis-cli",
			subdir: "redis",
			label: "Redis",
		}
	}

	/// The Valkey engine.
	pub fn valkey() -> RespEngine {
		RespEngine {
			db: DbType::Valkey,
			server_bin: "valkey-server",
			cli_bin: "valkey-cli",
			subdir: "valkey",
			label: "Valkey",
		}
	}

	fn detect_version(&self) -> Option<String> {
		let bin = find_binary(self.subdir, self.server_bin)?;
		let out = std::process::Command::new(&bin)
			.arg("--version")
			.output()
			.ok()?;
		parse_version(&String::from_utf8_lossy(&out.stdout))
	}
}

impl Engine for RespEngine {
	fn db_type(&self) -> DbType {
		self.db
	}

	fn versions(&self) -> Vec<Version> {
		let installed = self.detect_version();
		let bin_dir = find_binary(self.subdir, self.server_bin)
			.and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
			.unwrap_or_default();
		versions::families(self.db)
			.iter()
			.map(|&major| {
				let is_installed = installed
					.as_deref()
					.is_some_and(|v| versions::major_match(v, major));
				Version {
					db_type: self.db,
					major: major.to_string(),
					bin_path: if is_installed {
						bin_dir.clone()
					} else {
						String::new()
					},
					installed: is_installed,
					label: format!("{} {major}", self.label),
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
		if find_binary(self.subdir, self.server_bin).is_none() {
			return Err(EngineError::NotInstalled(format!(
				"{} not found — install it to create this instance",
				self.server_bin
			)));
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
		std::fs::write(&inst.conf_file, render_conf(inst))?;
		restrict(&inst.conf_file)?;
		Ok(())
	}

	fn start(&self, inst: &Instance) -> Result<(), EngineError> {
		let bin = find_binary(self.subdir, self.server_bin)
			.ok_or_else(|| EngineError::NotInstalled(self.server_bin.to_string()))?;
		let out = std::process::Command::new(&bin)
			.arg(&inst.conf_file)
			.output()?;
		if !out.status.success() {
			return Err(EngineError::Tooling(format!(
				"{} failed to start: {}",
				self.server_bin,
				String::from_utf8_lossy(&out.stderr).trim()
			)));
		}
		let deadline = Instant::now() + START_TIMEOUT;
		while Instant::now() < deadline {
			sleep(Duration::from_millis(200));
			if port_open(inst.port) {
				return Ok(());
			}
		}
		Err(EngineError::Tooling(format!(
			"{} did not open port {} within {}s",
			self.db.as_str(),
			inst.port,
			START_TIMEOUT.as_secs()
		)))
	}

	fn stop(&self, inst: &Instance) -> Result<(), EngineError> {
		if let Some(cli) = find_binary(self.subdir, self.cli_bin) {
			let ok = std::process::Command::new(&cli)
				.args(["-p", &inst.port.to_string(), "shutdown", "nosave"])
				.status()
				.map(|s| s.success())
				.unwrap_or(false);
			if ok {
				return Ok(());
			}
		}
		if signal_int(Path::new(&inst.pid_file)) {
			Ok(())
		} else {
			Err(EngineError::Tooling(
				"failed to signal the server process".into(),
			))
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

/// Renders the daemonized server config for an instance.
fn render_conf(inst: &Instance) -> String {
	format!(
		"port {}\nbind 127.0.0.1\ndaemonize yes\npidfile {}\nlogfile {}\ndir {}\nsave \"\"\n",
		inst.port, inst.pid_file, inst.log_file, inst.data_dir
	)
}

/// Parses a Redis/Valkey `--version` line, e.g. `… server v=7.4.0 sha=…`.
fn parse_version(out: &str) -> Option<String> {
	out.split_whitespace()
		.find_map(|f| f.strip_prefix("v="))
		.map(str::to_string)
}

fn parent_of(path: &str) -> &str {
	Path::new(path)
		.parent()
		.and_then(Path::to_str)
		.unwrap_or("")
}

#[cfg(unix)]
fn restrict(path: &str) -> Result<(), EngineError> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
	Ok(())
}

#[cfg(not(unix))]
fn restrict(_path: &str) -> Result<(), EngineError> {
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sample() -> Instance {
		Instance {
			id: "abc123".into(),
			name: "cache".into(),
			db_type: DbType::Redis,
			version: "7.4".into(),
			port: 6390,
			data_dir: "/base/data/abc123".into(),
			log_file: "/base/logs/abc123.log".into(),
			pid_file: "/base/pids/abc123.pid".into(),
			conf_file: "/base/conf/abc123.conf".into(),
			user: String::new(),
			status: Status::Stopped,
			created_at: "2026-06-29T00:00:00Z".into(),
			last_error: String::new(),
			upgrade_version: String::new(),
			patch_update: String::new(),
		}
	}

	#[test]
	fn redis_and_valkey_report_their_own_type() {
		assert_eq!(RespEngine::redis().db_type(), DbType::Redis);
		assert_eq!(RespEngine::valkey().db_type(), DbType::Valkey);
	}

	#[test]
	fn conf_binds_loopback_disables_persistence_and_points_at_instance_paths() {
		let conf = render_conf(&sample());
		assert!(conf.contains("port 6390"));
		assert!(conf.contains("bind 127.0.0.1"));
		assert!(conf.contains("daemonize yes"));
		assert!(conf.contains("save \"\""));
		assert!(conf.contains("pidfile /base/pids/abc123.pid"));
		assert!(conf.contains("dir /base/data/abc123"));
	}

	#[test]
	fn version_is_parsed_from_the_v_field() {
		assert_eq!(
			parse_version("Redis server v=7.4.0 sha=00000000 bits=64"),
			Some("7.4.0".to_string())
		);
		assert_eq!(
			parse_version("Valkey server v=8.0.1 sha=abc"),
			Some("8.0.1".to_string())
		);
		assert_eq!(parse_version("no version here"), None);
	}

	#[test]
	fn versions_list_covers_the_bundled_families() {
		let listed = RespEngine::redis().versions();
		assert_eq!(listed.len(), versions::families(DbType::Redis).len());
		assert!(listed.iter().all(|v| v.db_type == DbType::Redis));
		assert_eq!(listed[0].label, format!("Redis {}", listed[0].major));
	}
}
