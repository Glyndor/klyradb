//! On-demand engine installation.
//!
//! KlyraDB does not bundle or download engine binaries itself; it asks the
//! host's package manager to install them — `apt-get` via `pkexec` on Linux,
//! Homebrew on macOS. Output is streamed line by line so the UI can show
//! progress. Inside a snap the app is confined and cannot drive the host
//! package manager, so installation is refused with a clear message.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use crate::engine::paths::snap_dir;
use crate::engine::DbType;

/// The host package that provides an engine family.
///
/// Postgres is the only engine whose package is version-specific (one package
/// per major); the others install a single server package regardless of the
/// requested family.
pub fn package_name(db: DbType, version: &str) -> String {
	match db {
		DbType::Postgres => format!("postgresql-{version}"),
		DbType::Mysql => "mysql-server".to_string(),
		DbType::Mariadb => "mariadb-server".to_string(),
		DbType::Redis => "redis-server".to_string(),
		DbType::Valkey => "valkey".to_string(),
		DbType::Mongodb => "mongodb-org".to_string(),
	}
}

/// Installs the engine package for `db`/`version`, streaming each output line to
/// `emit`. Returns an error message suitable for the UI on failure.
///
/// Refuses inside a snap (the package manager is out of reach) and on platforms
/// without a supported package manager.
pub fn install(db: DbType, version: &str, emit: impl FnMut(&str)) -> Result<(), String> {
	if snap_dir().is_some() {
		return Err(
			"inside a snap, engines must come from the host — install it with your package manager"
				.into(),
		);
	}
	let pkg = package_name(db, version);
	let (program, args) = install_command(&pkg)?;
	stream(program, &args, emit)
}

/// The package-manager invocation for the current platform.
fn install_command(pkg: &str) -> Result<(&'static str, Vec<String>), String> {
	if cfg!(target_os = "linux") {
		Ok((
			"pkexec",
			vec![
				"apt-get".into(),
				"install".into(),
				"-y".into(),
				pkg.to_string(),
			],
		))
	} else if cfg!(target_os = "macos") {
		Ok(("brew", vec!["install".into(), pkg.to_string()]))
	} else {
		Err("automatic install is not supported on this platform".into())
	}
}

/// Runs a command, forwarding stdout lines to `emit`, and maps a non-zero exit
/// to an error.
fn stream(program: &str, args: &[String], mut emit: impl FnMut(&str)) -> Result<(), String> {
	let mut child = Command::new(program)
		.args(args)
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.map_err(|e| format!("{program}: {e}"))?;

	if let Some(out) = child.stdout.take() {
		for line in BufReader::new(out).lines().map_while(Result::ok) {
			emit(&line);
		}
	}

	let status = child.wait().map_err(|e| e.to_string())?;
	if status.success() {
		Ok(())
	} else {
		Err(format!("{program} exited with {status}"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn postgres_package_is_major_specific() {
		assert_eq!(package_name(DbType::Postgres, "17"), "postgresql-17");
	}

	#[test]
	fn other_engines_use_a_single_server_package() {
		assert_eq!(package_name(DbType::Mysql, "8.4"), "mysql-server");
		assert_eq!(package_name(DbType::Mariadb, "11.4"), "mariadb-server");
		assert_eq!(package_name(DbType::Redis, "7.4"), "redis-server");
		assert_eq!(package_name(DbType::Valkey, "8.1"), "valkey");
		assert_eq!(package_name(DbType::Mongodb, "8.0"), "mongodb-org");
	}
}
