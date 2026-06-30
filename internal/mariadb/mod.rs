//! MariaDB engine.
//!
//! MariaDB shares MySQL's tooling shape but its own binaries (`mariadbd`,
//! `mariadb-install-db`, `mariadb-admin`), falling back to the legacy `mysql*`
//! names where a build still uses them. Like MySQL it is initialized with
//! `--no-defaults` so the host configuration can never redirect it.

use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::engine::paths::{engines_dir, search_path, snap_dir};
use crate::engine::proc::{check_pid, port_open, signal_int};
use crate::engine::{DbType, Engine, EngineError, Instance, Status, Version};
use crate::versions;

/// How long to wait for the server to begin accepting connections.
const START_TIMEOUT: Duration = Duration::from_secs(45);

/// The MariaDB engine.
pub struct MariadbEngine;

impl MariadbEngine {
	/// Builds the MariaDB engine.
	pub fn new() -> MariadbEngine {
		MariadbEngine
	}

	fn detect_version(&self) -> Option<String> {
		let bin = find_mariadbd()?;
		let out = std::process::Command::new(&bin)
			.arg("--version")
			.output()
			.ok()?;
		let text = String::from_utf8_lossy(&out.stdout);
		// Only accept a genuine MariaDB build; a plain MySQL `mysqld` is not ours.
		if !text.contains("MariaDB") {
			return None;
		}
		parse_version(&text)
	}
}

impl Default for MariadbEngine {
	fn default() -> Self {
		MariadbEngine::new()
	}
}

impl Engine for MariadbEngine {
	fn db_type(&self) -> DbType {
		DbType::Mariadb
	}

	fn versions(&self) -> Vec<Version> {
		let installed = self.detect_version();
		let bin_dir = find_mariadbd()
			.and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
			.unwrap_or_default();
		versions::families(DbType::Mariadb)
			.iter()
			.map(|&major| {
				let is_installed = installed
					.as_deref()
					.is_some_and(|v| versions::major_match(v, major));
				Version {
					db_type: DbType::Mariadb,
					major: major.to_string(),
					bin_path: if is_installed {
						bin_dir.clone()
					} else {
						String::new()
					},
					installed: is_installed,
					label: format!("MariaDB {major}"),
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
		let mariadbd =
			find_mariadbd().ok_or_else(|| EngineError::NotInstalled("mariadbd".into()))?;
		let init = find_bin("mariadb-install-db")
			.or_else(|| find_bin("mysql_install_db"))
			.ok_or_else(|| EngineError::NotInstalled("mariadb-install-db".into()))?;
		make_dirs(inst)?;
		std::fs::write(&inst.conf_file, render_conf(inst))?;
		restrict_file(&inst.conf_file)?;

		let basedir = base_prefix(&mariadbd);
		let mut args = vec![
			"--no-defaults".to_string(),
			format!("--user={}", inst.user),
			format!("--datadir={}", inst.data_dir),
			format!("--basedir={basedir}"),
		];
		if let Some(snap) = snap_dir() {
			let prefix = snap.join("opt/klyra-mariadb");
			args.push(format!(
				"--lc-messages-dir={}/share/mysql",
				prefix.display()
			));
		}
		run(&init, &args, "mariadb-install-db")
	}

	fn start(&self, inst: &Instance) -> Result<(), EngineError> {
		let safe = find_bin("mariadbd-safe").or_else(|| find_bin("mysqld_safe"));
		if let Some(safe) = safe {
			let spawned = std::process::Command::new(&safe)
				.arg(format!("--defaults-file={}", inst.conf_file))
				.spawn();
			if spawned.is_ok() {
				return wait_ready(inst.port);
			}
		}
		let mariadbd =
			find_mariadbd().ok_or_else(|| EngineError::NotInstalled("mariadbd".into()))?;
		run(
			&mariadbd,
			&[
				format!("--defaults-file={}", inst.conf_file),
				"--daemonize".into(),
			],
			"mariadbd start",
		)?;
		wait_ready(inst.port)
	}

	fn stop(&self, inst: &Instance) -> Result<(), EngineError> {
		let sock = Path::new(&inst.data_dir).join("mariadb.sock");
		for admin in ["mariadb-admin", "mysqladmin"] {
			if let Some(bin) = find_bin(admin) {
				let ok = std::process::Command::new(&bin)
					.args([
						"-u",
						"root",
						&format!("--socket={}", sock.display()),
						"shutdown",
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
			Err(EngineError::Tooling("failed to signal mariadbd".into()))
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

/// Resolves the MariaDB daemon, validating it is a genuine MariaDB build (not a
/// MySQL `mysqld` masquerading under the same name).
fn find_mariadbd() -> Option<PathBuf> {
	for name in ["mariadbd", "mysqld"] {
		if let Some(bin) = find_bin(name) {
			let out = std::process::Command::new(&bin)
				.arg("--version")
				.output()
				.ok();
			if let Some(out) = out {
				if String::from_utf8_lossy(&out.stdout).contains("MariaDB") {
					return Some(bin);
				}
			}
		}
	}
	None
}

/// Resolves a MariaDB executable across the snap prefix or the system paths.
fn find_bin(name: &str) -> Option<PathBuf> {
	if let Some(snap) = snap_dir() {
		for rel in ["opt/klyra-mariadb/sbin", "opt/klyra-mariadb/bin"] {
			let p = snap.join(rel).join(name);
			if p.is_file() {
				return Some(p);
			}
		}
		return None;
	}
	let eng = engines_dir();
	let candidates = [
		eng.join("mariadb/sbin").join(name),
		eng.join("mariadb/bin").join(name),
		PathBuf::from("/usr/sbin").join(name),
		PathBuf::from("/usr/bin").join(name),
		PathBuf::from("/usr/local/sbin").join(name),
		PathBuf::from("/usr/local/bin").join(name),
	];
	candidates
		.into_iter()
		.find(|p| p.is_file())
		.or_else(|| search_path(name))
}

/// The install base directory: the snap prefix when confined, otherwise the
/// grandparent of the daemon binary (e.g. `/usr/sbin/mariadbd` → `/usr`).
fn base_prefix(mariadbd: &Path) -> String {
	if let Some(snap) = snap_dir() {
		return snap
			.join("opt/klyra-mariadb")
			.to_string_lossy()
			.into_owned();
	}
	mariadbd
		.parent()
		.and_then(Path::parent)
		.map(|p| p.to_string_lossy().into_owned())
		.unwrap_or_else(|| "/usr".to_string())
}

fn make_dirs(inst: &Instance) -> Result<(), EngineError> {
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
	restrict_dir(&inst.data_dir)
}

/// Renders the `[mariadbd]` config for an instance, bound to loopback.
fn render_conf(inst: &Instance) -> String {
	let sock = Path::new(&inst.data_dir).join("mariadb.sock");
	format!(
		"[mariadbd]\ndatadir = {}\nsocket = {}\nport = {}\npid-file = {}\nlog-error = {}\nuser = {}\nbind-address = 127.0.0.1\n",
		inst.data_dir,
		sock.display(),
		inst.port,
		inst.pid_file,
		inst.log_file,
		inst.user
	)
}

/// Parses a `mariadbd --version` line: the token after `Ver`, before `-suffix`.
fn parse_version(out: &str) -> Option<String> {
	let fields: Vec<&str> = out.split_whitespace().collect();
	for (i, f) in fields.iter().enumerate() {
		if f.eq_ignore_ascii_case("ver") {
			return fields
				.get(i + 1)
				.map(|v| v.split('-').next().unwrap_or(v).to_string());
		}
	}
	None
}

fn wait_ready(port: u16) -> Result<(), EngineError> {
	let deadline = Instant::now() + START_TIMEOUT;
	while Instant::now() < deadline {
		sleep(Duration::from_millis(500));
		if port_open(port) {
			return Ok(());
		}
	}
	Err(EngineError::Tooling(format!(
		"mariadb did not open port {port} within {}s",
		START_TIMEOUT.as_secs()
	)))
}

fn run(program: &Path, args: &[String], what: &str) -> Result<(), EngineError> {
	let out = std::process::Command::new(program).args(args).output()?;
	if out.status.success() {
		return Ok(());
	}
	Err(EngineError::Tooling(format!(
		"{what}: {}",
		String::from_utf8_lossy(&out.stderr).trim()
	)))
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
			id: "ma0001".into(),
			name: "app".into(),
			db_type: DbType::Mariadb,
			version: "11.4".into(),
			port: 3316,
			data_dir: "/base/data/ma0001".into(),
			log_file: "/base/logs/ma0001.log".into(),
			pid_file: "/base/pids/ma0001.pid".into(),
			conf_file: "/base/conf/ma0001.cnf".into(),
			user: "root".into(),
			status: Status::Stopped,
			created_at: "2026-06-29T00:00:00Z".into(),
			last_error: String::new(),
			upgrade_version: String::new(),
			patch_update: String::new(),
		}
	}

	#[test]
	fn reports_mariadb_type() {
		assert_eq!(MariadbEngine::new().db_type(), DbType::Mariadb);
	}

	#[test]
	fn conf_uses_mariadbd_section_bound_to_loopback() {
		let conf = render_conf(&sample());
		assert!(conf.contains("[mariadbd]"));
		assert!(conf.contains("port = 3316"));
		assert!(conf.contains("socket = /base/data/ma0001/mariadb.sock"));
		assert!(conf.contains("bind-address = 127.0.0.1"));
	}

	#[test]
	fn version_is_parsed_from_a_mariadb_build_string() {
		let out = "mariadbd  Ver 11.4.2-MariaDB-1 for debian-linux-gnu";
		assert_eq!(parse_version(out), Some("11.4.2".to_string()));
	}

	#[test]
	fn base_prefix_is_grandparent_of_binary_outside_snap() {
		// No SNAP in the test env, so it derives from the path.
		if snap_dir().is_none() {
			assert_eq!(base_prefix(Path::new("/usr/sbin/mariadbd")), "/usr");
		}
	}

	#[test]
	fn versions_list_covers_the_bundled_families() {
		let listed = MariadbEngine::new().versions();
		assert_eq!(listed.len(), versions::families(DbType::Mariadb).len());
		assert_eq!(listed[0].label, format!("MariaDB {}", listed[0].major));
	}
}
