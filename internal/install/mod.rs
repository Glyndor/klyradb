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

/// A user-facing explanation of why installation is impossible on Linux for
/// the given engine and version, or `None` if installation is supported.
///
/// Returned only for engines the host package manager cannot supply: MongoDB
/// today, because the `mongodb` package was removed from Debian/Ubuntu years
/// ago and MongoDB Inc. distributes the official server as `mongodb-org` from
/// `repo.mongodb.org`, a repository klyradb does not configure. Surfaced by
/// [`install`] as the install error so the user sees the real reason instead
/// of a raw apt "Unable to locate package" message, and consulted by the
/// manager before the Snap branch so Snap users see the same specific reason.
pub fn blocked_reason(db: DbType, _version: &str) -> Option<String> {
	match db {
		DbType::Mongodb => Some(
			"MongoDB on Linux is not installable from klyradb. The package \"mongodb\" was removed from Debian and Ubuntu years ago; MongoDB Inc. distributes the official server as \"mongodb-org\" from repo.mongodb.org, a repository klyradb does not configure. To use MongoDB on Linux today: either add the official MongoDB APT repository to your system (https://www.mongodb.com/docs/manual/installation/), or wait for the verified download path in the upcoming rewrite."
				.into(),
		),
		_ => None,
	}
}

/// Installs the engine package for `db`/`version`, streaming each output line to
/// `emit`. Returns an error message suitable for the UI on failure.
///
/// The Linux-specific [`blocked_reason`] check runs **before** the Snap
/// branch: on both Linux direct downloads and under Snap, a user clicking
/// Install on a MongoDB instance sees the real reason (apt/brew does not
/// ship MongoDB) rather than the generic "not bundled in the snap package"
/// message. Refuses inside a snap for every other engine, and on platforms
/// without a supported package manager.
pub fn install(db: DbType, version: &str, emit: impl FnMut(&str)) -> Result<(), String> {
	// Linux-specific blocks take precedence over the Snap branch below so the
	// user sees the real reason (apt/brew does not ship MongoDB) instead of
	// the generic "not bundled in the snap package" message on Linux direct
	// downloads AND under Snap.
	if let Some(reason) = blocked_reason(db, version) {
		return Err(reason);
	}
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

	#[test]
	fn blocked_reason_explains_mongodb_on_linux() {
		let reason = blocked_reason(DbType::Mongodb, "8.2.6").expect("MongoDB on Linux is blocked");
		// The message must answer the three questions: what, why, what-to-do.
		for needle in [
			"MongoDB on Linux",
			"repo.mongodb.org", // where the official package lives
			"klyradb does not", // why klyradb can't do it for you
			"mongodb-org",      // the right package name
			"rewrite",          // the upcoming verified-download path
		] {
			assert!(
				reason.contains(needle),
				"MongoDB reason missing {needle:?}; full message:\n{reason}"
			);
		}
	}

	#[test]
	fn blocked_reason_is_none_for_every_other_engine() {
		for db in [
			DbType::Postgres,
			DbType::Mysql,
			DbType::Mariadb,
			DbType::Valkey,
			DbType::Redis,
		] {
			assert_eq!(
				blocked_reason(db, ""),
				None,
				"blocked_reason({db:?}) must be None — install is supported",
			);
		}
	}

	#[test]
	fn install_mongodb_on_linux_short_circuits_before_the_snap_branch() {
		// Simulate a snap runtime so the snap branch would otherwise trigger.
		// The blocked reason must win, not the generic snap message.
		// SAFETY: tests in this module are not run concurrently with other
		// tests that read SNAP (none currently exist).
		unsafe {
			std::env::set_var("SNAP", "/snap/klyradb/x1");
		}
		let result = install(DbType::Mongodb, "8.2.6", |_| {});
		unsafe {
			std::env::remove_var("SNAP");
		}
		let err = result.expect_err("Install(MongoDB,Linux) must fail");
		assert!(
			err.contains("MongoDB on Linux"),
			"got the snap message instead of the blocked reason: {err}",
		);
		assert!(
			!err.contains("snap"),
			"install must short-circuit before the Snap branch — got a snap message: {err}",
		);
	}

	#[test]
	fn install_non_blocked_engine_inside_a_snap_uses_the_snap_branch() {
		// Postgres is not blocked; under SNAP the snap branch must trigger,
		// not the blocked reason (which is None).
		unsafe {
			std::env::set_var("SNAP", "/snap/klyradb/x1");
		}
		let result = install(DbType::Postgres, "17", |_| {});
		unsafe {
			std::env::remove_var("SNAP");
		}
		let err = result.expect_err("Install(Postgres, snap) must fail with the snap message");
		assert!(
			err.contains("snap"),
			"non-blocked engine under snap must surface the snap message, got: {err}",
		);
		assert!(
			!err.contains("MongoDB"),
			"non-blocked engine must not surface the MongoDB reason: {err}",
		);
	}
}
