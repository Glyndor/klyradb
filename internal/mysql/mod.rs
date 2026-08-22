//! MySQL engine.
//!
//! Instances are initialized insecurely (no root password) for local-only use
//! and always with `--no-defaults`, so MySQL never reads the host's
//! `/etc/mysql/*` and cannot be redirected to a shared datadir or host plugins.
//! Inside a snap the isolated `opt/klyra-mysql` prefix is supplied explicitly.

use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::engine::paths::{engines_dir, search_path, snap_dir};
use crate::engine::proc::{check_pid, port_open, signal_int};
use crate::engine::{DbType, Engine, EngineError, Instance, Status, Version};
use crate::versions;

/// How long to wait for the server to begin accepting connections.
const START_TIMEOUT: Duration = Duration::from_secs(45);

/// The MySQL engine.
pub struct MysqlEngine;

impl MysqlEngine {
	/// Builds the MySQL engine.
	pub fn new() -> MysqlEngine {
		MysqlEngine
	}

	fn detect_version(&self) -> Option<String> {
		let bin = find_bin("mysqld")?;
		let out = std::process::Command::new(&bin)
			.arg("--version")
			.output()
			.ok()?;
		let text = String::from_utf8_lossy(&out.stdout);
		// MariaDB also ships a `mysqld`; its version string says "MariaDB", so
		// reject it here — that engine is managed separately.
		if text.contains("MariaDB") {
			return None;
		}
		parse_version(&text)
	}
}

impl Default for MysqlEngine {
	fn default() -> Self {
		MysqlEngine::new()
	}
}

impl Engine for MysqlEngine {
	fn db_type(&self) -> DbType {
		DbType::Mysql
	}

	fn versions(&self) -> Vec<Version> {
		let installed = self.detect_version();
		let bin_dir = find_bin("mysqld")
			.and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
			.unwrap_or_default();
		versions::families(DbType::Mysql)
			.iter()
			.map(|&major| {
				let is_installed = installed
					.as_deref()
					.is_some_and(|v| versions::major_match(v, major));
				Version {
					db_type: DbType::Mysql,
					major: major.to_string(),
					bin_path: if is_installed {
						bin_dir.clone()
					} else {
						String::new()
					},
					installed: is_installed,
					label: format!("MySQL {major}"),
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
		let mysqld =
			find_bin("mysqld").ok_or_else(|| EngineError::NotInstalled("mysqld".into()))?;
		make_dirs(inst)?;
		std::fs::write(&inst.conf_file, render_conf(inst))?;
		restrict_file(&inst.conf_file)?;

		let mut args = vec![
			"--no-defaults".to_string(),
			"--initialize-insecure".to_string(),
			format!("--user={}", inst.user),
			format!("--datadir={}", inst.data_dir),
		];
		if let Some(base) = snap_prefix() {
			args.push(format!("--basedir={base}"));
			args.push(format!("--plugin-dir={base}/lib/mysql/plugin"));
			args.push(format!("--lc-messages-dir={base}/share/mysql"));
		}
		run(&mysqld, &args, "mysql initialize")
	}

	fn start(&self, inst: &Instance) -> Result<(), EngineError> {
		if let Some(safe) = find_bin("mysqld_safe") {
			let spawned = std::process::Command::new(&safe)
				.arg(format!("--defaults-file={}", inst.conf_file))
				.spawn();
			if spawned.is_ok() {
				return wait_ready(inst.port);
			}
		}
		let mysqld =
			find_bin("mysqld").ok_or_else(|| EngineError::NotInstalled("mysqld".into()))?;
		run(
			&mysqld,
			&[
				format!("--defaults-file={}", inst.conf_file),
				"--daemonize".into(),
			],
			"mysqld start",
		)?;
		wait_ready(inst.port)
	}

	fn stop(&self, inst: &Instance) -> Result<(), EngineError> {
		if let Some(admin) = find_bin("mysqladmin") {
			let sock = Path::new(&inst.data_dir).join("mysql.sock");
			let ok = std::process::Command::new(&admin)
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
		if signal_int(Path::new(&inst.pid_file)) {
			Ok(())
		} else {
			Err(EngineError::Tooling("failed to signal mysqld".into()))
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

/// Resolves a MySQL executable, honouring snap confinement and the `sbin`/`bin`
/// split MySQL distributions use.
fn find_bin(name: &str) -> Option<PathBuf> {
	if let Some(snap) = snap_dir() {
		for rel in ["opt/klyra-mysql/sbin", "opt/klyra-mysql/bin"] {
			let p = snap.join(rel).join(name);
			if p.is_file() {
				return Some(p);
			}
		}
		return None;
	}
	let eng = engines_dir();
	let candidates = [
		eng.join("mysql/sbin").join(name),
		eng.join("mysql/bin").join(name),
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

/// The isolated MySQL prefix inside a snap, as a string, if confined.
fn snap_prefix() -> Option<String> {
	snap_dir().map(|s| s.join("opt/klyra-mysql").to_string_lossy().into_owned())
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

/// Renders the `[mysqld]` config for an instance, bound to loopback.
fn render_conf(inst: &Instance) -> String {
	// Unix-socket path is always slash-separated (this is a Linux engine).
	let sock = format!("{}/mysql.sock", inst.data_dir);
	let mut conf = format!(
		"[mysqld]\ndatadir = {}\nsocket = {}\nport = {}\npid-file = {}\nlog-error = {}\nuser = {}\nbind-address = 127.0.0.1\n",
		inst.data_dir,
		sock,
		inst.port,
		inst.pid_file,
		inst.log_file,
		inst.user
	);
	if let Some(base) = snap_prefix() {
		conf.push_str(&format!("basedir = {base}\n"));
		conf.push_str(&format!("plugin-dir = {base}/lib/mysql/plugin\n"));
		conf.push_str(&format!("lc-messages-dir = {base}/share/mysql\n"));
	}
	conf
}

/// Parses a `mysqld --version` line: the token after `Ver`, before any `-suffix`.
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
		"mysql did not open port {port} within {}s",
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
			id: "my0001".into(),
			name: "app".into(),
			db_type: DbType::Mysql,
			version: "8.4".into(),
			port: 3306,
			data_dir: "/base/data/my0001".into(),
			log_file: "/base/logs/my0001.log".into(),
			pid_file: "/base/pids/my0001.pid".into(),
			conf_file: "/base/conf/my0001.cnf".into(),
			user: "root".into(),
			status: Status::Stopped,
			created_at: "2026-06-29T00:00:00Z".into(),
			last_error: String::new(),
			upgrade_version: String::new(),
			patch_update: String::new(),
		}
	}

	#[test]
	fn reports_mysql_type() {
		assert_eq!(MysqlEngine::new().db_type(), DbType::Mysql);
	}

	#[test]
	fn conf_binds_loopback_with_instance_paths() {
		let conf = render_conf(&sample());
		assert!(conf.contains("[mysqld]"));
		assert!(conf.contains("datadir = /base/data/my0001"));
		assert!(conf.contains("port = 3306"));
		assert!(conf.contains("socket = /base/data/my0001/mysql.sock"));
		assert!(conf.contains("bind-address = 127.0.0.1"));
	}

	#[test]
	fn version_is_parsed_after_ver_and_before_suffix() {
		let out = "/usr/sbin/mysqld  Ver 8.4.0-1ubuntu for Linux";
		assert_eq!(parse_version(out), Some("8.4.0".to_string()));
		assert_eq!(parse_version("no version here"), None);
	}

	#[test]
	fn versions_list_covers_the_bundled_families() {
		let listed = MysqlEngine::new().versions();
		assert_eq!(listed.len(), versions::families(DbType::Mysql).len());
		assert_eq!(listed[0].label, format!("MySQL {}", listed[0].major));
	}
}
