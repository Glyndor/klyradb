//! Unit tests for the instance manager.
//!
//! Split into its own file to keep `mod.rs` under the line limit; a child
//! module still sees the parent's private items through `super`.

use super::*;
use std::sync::{Arc, Mutex};

/// An engine stub that records calls and can be told to fail on start.
struct FakeEngine {
	db: DbType,
	fail_start: bool,
	started: Arc<Mutex<Vec<String>>>,
}

impl Engine for FakeEngine {
	fn db_type(&self) -> DbType {
		self.db
	}
	fn versions(&self) -> Vec<crate::engine::Version> {
		vec![Version {
			db_type: self.db,
			major: "1".into(),
			bin_path: String::new(),
			installed: false,
			label: "Fake 1".into(),
			latest_patch: "1".into(),
			installed_version: String::new(),
		}]
	}
	fn create(&self, _inst: &mut Instance) -> Result<(), EngineError> {
		Ok(())
	}
	fn start(&self, inst: &Instance) -> Result<(), EngineError> {
		if self.fail_start {
			return Err(EngineError::Tooling("boom".into()));
		}
		self.started.lock().unwrap().push(inst.id.clone());
		Ok(())
	}
	fn stop(&self, _inst: &Instance) -> Result<(), EngineError> {
		Ok(())
	}
	fn delete(&self, inst: &Instance) -> Result<(), EngineError> {
		std::fs::remove_dir_all(&inst.data_dir).ok();
		Ok(())
	}
	fn check_status(&self, inst: &Instance) -> Status {
		inst.status
	}
}

fn manager_with(
	db: DbType,
	fail_start: bool,
) -> (Manager, tempfile::TempDir, Arc<Mutex<Vec<String>>>) {
	let dir = tempfile::tempdir().unwrap();
	let started = Arc::new(Mutex::new(Vec::new()));
	let engine = FakeEngine {
		db,
		fail_start,
		started: started.clone(),
	};
	let mgr = Manager::new(dir.path(), vec![Box::new(engine)]).unwrap();
	(mgr, dir, started)
}

#[test]
fn create_rejects_empty_name() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	assert!(matches!(
		m.create("  ", DbType::Postgres, "17", None),
		Err(ManagerError::Invalid(_))
	));
}

#[test]
fn create_rejects_unknown_engine() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	assert!(matches!(
		m.create("dev", DbType::Mysql, "8.4", None),
		Err(ManagerError::Invalid(_))
	));
}

#[test]
fn create_lays_out_isolated_paths_and_persists() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	let b = m.create("b", DbType::Postgres, "17", None).unwrap();
	assert_ne!(a.id, b.id);
	assert_ne!(a.data_dir, b.data_dir);
	assert_ne!(a.port, b.port, "two instances must not share a port");
	assert_eq!(a.status, Status::Stopped);
	// A reload sees both persisted.
	m.load_all().unwrap();
	assert_eq!(m.list().len(), 2);
}

#[test]
fn requested_busy_port_is_rejected() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	assert!(matches!(
		m.create("b", DbType::Postgres, "17", Some(a.port)),
		Err(ManagerError::Invalid(_))
	));
}

#[test]
fn failed_start_records_error_status_and_message() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, true);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	let err = m.start(&a.id).unwrap_err();
	assert!(matches!(err, ManagerError::Engine(_)));
	let reloaded = m.list().into_iter().find(|i| i.id == a.id).unwrap();
	assert_eq!(reloaded.status, Status::Error);
	assert_eq!(reloaded.last_error, "boom");
}

#[test]
fn delete_removes_from_registry_and_disk() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	m.delete(&a.id).unwrap();
	assert!(m.list().is_empty());
	assert!(matches!(m.status(&a.id), Err(ManagerError::NotFound(_))));
}

#[test]
fn next_free_port_starts_at_engine_default() {
	let (m, _d, _s) = manager_with(DbType::Mongodb, false);
	let p = m.next_free_port(DbType::Mongodb).unwrap();
	assert!(p >= default_port(DbType::Mongodb));
}

#[test]
fn start_success_marks_running_and_records_the_call() {
	let (mut m, _d, started) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	m.start(&a.id).unwrap();
	let inst = m.get(&a.id).unwrap();
	assert_eq!(inst.status, Status::Running);
	assert!(inst.last_error.is_empty());
	assert_eq!(started.lock().unwrap().as_slice(), &[a.id]);
}

#[test]
fn stop_marks_stopped() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	m.start(&a.id).unwrap();
	m.stop(&a.id).unwrap();
	assert_eq!(m.get(&a.id).unwrap().status, Status::Stopped);
}

#[test]
fn stop_all_stops_every_instance() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	let b = m.create("b", DbType::Postgres, "17", None).unwrap();
	m.start(&a.id).unwrap();
	m.start(&b.id).unwrap();
	m.stop_all();
	assert!(m.list().iter().all(|i| i.status == Status::Stopped));
}

#[test]
fn lifecycle_calls_on_a_missing_instance_report_not_found() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	assert!(matches!(m.start("nope"), Err(ManagerError::NotFound(_))));
	assert!(matches!(m.stop("nope"), Err(ManagerError::NotFound(_))));
	assert!(matches!(m.delete("nope"), Err(ManagerError::NotFound(_))));
	assert!(matches!(m.status("nope"), Err(ManagerError::NotFound(_))));
}

#[test]
fn status_reflects_the_engine_check() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	// The fake echoes the stored status, which create left as Stopped.
	assert_eq!(m.status(&a.id).unwrap(), Status::Stopped);
}

#[test]
fn create_honours_an_explicit_free_port() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let inst = m.create("a", DbType::Postgres, "17", Some(15432)).unwrap();
	assert_eq!(inst.port, 15432);
}

#[test]
fn versions_aggregates_every_engine() {
	let (m, _d, _s) = manager_with(DbType::Postgres, false);
	let vs = m.versions();
	assert_eq!(vs.len(), 1);
	assert_eq!(vs[0].db_type, DbType::Postgres);
}

#[test]
fn get_returns_a_clone_or_none() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	assert_eq!(m.get(&a.id).unwrap().id, a.id);
	assert!(m.get("missing").is_none());
}

#[test]
fn load_all_repopulates_from_disk() {
	let dir = tempfile::tempdir().unwrap();
	let id = {
		let started = Arc::new(Mutex::new(Vec::new()));
		let engine = FakeEngine {
			db: DbType::Postgres,
			fail_start: false,
			started,
		};
		let mut m = Manager::new(dir.path(), vec![Box::new(engine)]).unwrap();
		m.create("a", DbType::Postgres, "17", None).unwrap().id
	};
	// A fresh manager over the same base dir sees the persisted instance.
	let engine = FakeEngine {
		db: DbType::Postgres,
		fail_start: false,
		started: Arc::new(Mutex::new(Vec::new())),
	};
	let mut m2 = Manager::new(dir.path(), vec![Box::new(engine)]).unwrap();
	m2.load_all().unwrap();
	assert_eq!(m2.get(&id).unwrap().id, id);
	assert_eq!(m2.base_dir(), dir.path());
}

#[test]
fn install_mongodb_returns_blocked_reason_and_marks_needs_install() {
	// MongoDB is a real DbType the Rust rewrite wires an engine for, but the
	// fake-only manager here exercises the manager-side blocked path: the
	// install command must short-circuit BEFORE any install command runs and
	// MUST leave the instance in NeedsInstall with the explanatory reason on
	// disk so the frontend can show it. Use the real install::blocked_reason
	// message verbatim so a future refactor that drops it fails the test.
	let (mut m, _d, _s) = manager_with(DbType::Mongodb, false);
	let a = m.create("a", DbType::Mongodb, "8.2.6", None).unwrap();

	let err = m.install(&a.id, |_| {}).unwrap_err();
	let msg = match err {
		ManagerError::Engine(m) => m,
		other => panic!("expected ManagerError::Engine, got {other:?}"),
	};
	// The message is the blocked reason, not a snap/brew/manager-of-something
	// else message.
	assert!(
		msg.contains("MongoDB on Linux"),
		"install must surface the MongoDB-on-Linux reason, got: {msg}",
	);
	assert!(
		!msg.contains("Unable to locate package"),
		"install must not reach apt — got a raw apt message: {msg}",
	);
	assert!(
		!msg.contains("brew"),
		"install must not reach brew — got a brew message: {msg}",
	);
	// The persisted instance carries the reason so the frontend card can show
	// it. Status becomes NeedsInstall, last_error matches the manager's error.
	let stored = m.get(&a.id).unwrap();
	assert_eq!(stored.status, Status::NeedsInstall);
	assert_eq!(stored.last_error, msg);
	// Reloading from disk sees the same status and reason.
	let mut m2 = Manager::new(
		m.base_dir(),
		vec![Box::new(FakeEngine {
			db: DbType::Mongodb,
			fail_start: false,
			started: Arc::new(Mutex::new(Vec::new())),
		})],
	)
	.unwrap();
	m2.load_all().unwrap();
	let reloaded = m2.get(&a.id).unwrap();
	assert_eq!(reloaded.status, Status::NeedsInstall);
	assert_eq!(reloaded.last_error, msg);
}

#[test]
fn install_mongodb_takes_precedence_over_the_snap_branch() {
	// With SNAP set, the install::install snap branch would otherwise trigger
	// for any engine. The blocked check on the manager side must still win
	// for MongoDB — the user sees the real reason, not the snap message.
	let (mut m, _d, _s) = manager_with(DbType::Mongodb, false);
	let a = m.create("a", DbType::Mongodb, "8.2.6", None).unwrap();
	unsafe {
		std::env::set_var("SNAP", "/snap/klyradb/x1");
	}
	let result = m.install(&a.id, |_| {});
	unsafe {
		std::env::remove_var("SNAP");
	}
	let err = result.unwrap_err();
	let msg = match err {
		ManagerError::Engine(m) => m,
		other => panic!("expected ManagerError::Engine, got {other:?}"),
	};
	assert!(
		msg.contains("MongoDB on Linux"),
		"blocked reason must win over the Snap branch, got: {msg}",
	);
	assert!(
		!msg.contains("not bundled") && !msg.contains("snap"),
		"the snap-branch message must not leak through: {msg}",
	);
}

#[test]
fn upgrade_patch_mongodb_returns_blocked_reason_and_marks_needs_install() {
	let (mut m, _d, _s) = manager_with(DbType::Mongodb, false);
	let a = m.create("a", DbType::Mongodb, "8.2.6", None).unwrap();
	let err = m.upgrade_patch(&a.id, |_| {}).unwrap_err();
	let msg = match err {
		ManagerError::Engine(m) => m,
		other => panic!("expected ManagerError::Engine, got {other:?}"),
	};
	assert!(
		msg.contains("MongoDB on Linux"),
		"upgrade_patch must surface the MongoDB-on-Linux reason, got: {msg}",
	);
	let stored = m.get(&a.id).unwrap();
	assert_eq!(stored.status, Status::NeedsInstall);
	assert_eq!(stored.last_error, msg);
}

#[test]
fn install_non_blocked_engine_under_snap_falls_through_to_install() {
	// Postgres is not blocked, so the blocked check returns None and the
	// install command runs. Under SNAP it hits the snap branch and returns
	// the snap message — which is exactly what the user should see for a
	// non-blocked engine. This is the inverse of the MongoDB precedence test.
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	let a = m.create("a", DbType::Postgres, "17", None).unwrap();
	unsafe {
		std::env::set_var("SNAP", "/snap/klyradb/x1");
	}
	let result = m.install(&a.id, |_| {});
	unsafe {
		std::env::remove_var("SNAP");
	}
	let err = result.unwrap_err();
	let msg = match err {
		ManagerError::Engine(m) => m,
		other => panic!("expected ManagerError::Engine, got {other:?}"),
	};
	assert!(
		msg.contains("snap"),
		"non-blocked engine under snap must reach the snap branch: {msg}",
	);
	assert!(
		!msg.contains("MongoDB"),
		"non-blocked engine must not surface the MongoDB reason: {msg}",
	);
}

#[test]
fn install_unknown_instance_returns_not_found() {
	let (mut m, _d, _s) = manager_with(DbType::Postgres, false);
	assert!(matches!(
		m.install("missing", |_| {}),
		Err(ManagerError::NotFound(_))
	));
	assert!(matches!(
		m.upgrade_patch("missing", |_| {}),
		Err(ManagerError::NotFound(_))
	));
}
