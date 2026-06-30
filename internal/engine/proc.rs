//! Process liveness and TCP port probing for engine status checks.
//!
//! Status is derived from two independent signals — a live pid and an open port
//! — because either alone lies: a stale pidfile outlives a crashed process, and
//! a port can be briefly closed while a healthy server reloads. An instance is
//! considered up when *either* holds.

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

/// How long to wait for a TCP connection before deeming the port closed.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

/// Returns the pid recorded in `pid_file` if that process is currently alive.
///
/// Only the first line is read, so Postgres' multi-line `postmaster.pid` works.
/// Liveness is checked via `/proc/<pid>`; a missing or malformed file, or a dead
/// process, yields `None`.
pub fn check_pid(pid_file: &Path) -> Option<u32> {
	let data = std::fs::read_to_string(pid_file).ok()?;
	let first = data.trim().lines().next()?.trim();
	let pid: u32 = first.parse().ok()?;
	if pid == 0 {
		return None;
	}
	Path::new(&format!("/proc/{pid}")).exists().then_some(pid)
}

/// Whether something is accepting TCP connections on loopback `port`.
pub fn port_open(port: u16) -> bool {
	let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
	TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).is_ok()
}

/// Asks the process recorded in `pid_file` to shut down with SIGINT.
///
/// Shells out to `kill` rather than calling `libc::kill` so the crate stays free
/// of `unsafe`. A missing pidfile or dead process is a no-op success, since the
/// goal — the process not running — already holds.
pub fn signal_int(pid_file: &Path) -> bool {
	let Some(pid) = check_pid(pid_file) else {
		return true;
	};
	std::process::Command::new("kill")
		.arg("-INT")
		.arg(pid.to_string())
		.status()
		.map(|s| s.success())
		.unwrap_or(false)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;

	#[test]
	fn missing_pidfile_yields_none() {
		assert_eq!(check_pid(Path::new("/nonexistent/pid")), None);
	}

	#[test]
	fn malformed_pidfile_yields_none() {
		let mut f = tempfile::NamedTempFile::new().unwrap();
		write!(f, "not-a-number").unwrap();
		assert_eq!(check_pid(f.path()), None);
	}

	#[test]
	fn dead_pid_is_not_reported_alive() {
		// PID 0 is never a real process and is rejected outright.
		let mut f = tempfile::NamedTempFile::new().unwrap();
		writeln!(f, "0").unwrap();
		assert_eq!(check_pid(f.path()), None);
	}

	#[test]
	fn own_pid_reads_back_as_alive() {
		let mut f = tempfile::NamedTempFile::new().unwrap();
		// Multi-line, like postmaster.pid — only the first line counts.
		writeln!(f, "{}", std::process::id()).unwrap();
		writeln!(f, "/some/data/dir").unwrap();
		assert_eq!(check_pid(f.path()), Some(std::process::id()));
	}

	#[test]
	fn signal_int_on_missing_pidfile_is_a_noop_success() {
		assert!(signal_int(Path::new("/nonexistent/pid")));
	}

	#[test]
	fn unbound_high_port_reads_as_closed() {
		// Nothing is expected to listen here during tests.
		assert!(!port_open(1));
	}
}
