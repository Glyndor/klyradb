//! Instance manager — the orchestrator the bridge drives.
//!
//! Owns the registry of managed instances, their on-disk layout and the
//! lifecycle calls that fan out to the per-engine implementations. The manager
//! guarantees the invariants a correct instance manager must hold: every
//! instance gets an isolated data directory, no two instances are handed the
//! same port, and the persisted registry is always consistent with what is in
//! memory.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use crate::engine::{DbType, Engine, EngineError, Instance, Status, Version};
use crate::store::{Store, StoreError};

/// The first port each engine kind is offered; allocation scans upward from
/// here. Kept distinct per engine so fresh instances rarely collide on creation.
fn default_port(db: DbType) -> u16 {
	match db {
		DbType::Postgres => 5432,
		DbType::Mysql => 3306,
		DbType::Mariadb => 3316,
		DbType::Valkey => 6379,
		DbType::Redis => 6390,
		DbType::Mongodb => 27017,
	}
}

/// How far above the default port allocation will scan before giving up.
const PORT_SCAN_SPAN: u16 = 500;

/// A manager-level failure.
#[derive(Debug)]
pub enum ManagerError {
	/// The requested instance id is not in the registry.
	NotFound(String),
	/// A supplied argument was invalid (empty name, unknown engine, …).
	Invalid(String),
	/// No free port was found in the scan window.
	NoFreePort(DbType),
	/// An engine operation failed.
	Engine(String),
	/// Reading or writing the registry failed.
	Store(String),
}

impl std::fmt::Display for ManagerError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ManagerError::NotFound(id) => write!(f, "no such instance: {id}"),
			ManagerError::Invalid(m) => write!(f, "{m}"),
			ManagerError::NoFreePort(db) => {
				write!(f, "no free port for {} in scan window", db.as_str())
			}
			ManagerError::Engine(m) => write!(f, "{m}"),
			ManagerError::Store(m) => write!(f, "{m}"),
		}
	}
}

impl std::error::Error for ManagerError {}

impl From<EngineError> for ManagerError {
	fn from(e: EngineError) -> Self {
		ManagerError::Engine(e.to_string())
	}
}

impl From<StoreError> for ManagerError {
	fn from(e: StoreError) -> Self {
		ManagerError::Store(e.to_string())
	}
}

/// The set of per-instance directories under the base directory.
struct Layout {
	data: PathBuf,
	logs: PathBuf,
	pids: PathBuf,
	conf: PathBuf,
}

/// The instance registry and lifecycle orchestrator.
pub struct Manager {
	base: PathBuf,
	layout: Layout,
	store: Store,
	engines: HashMap<DbType, Box<dyn Engine>>,
	instances: HashMap<String, Instance>,
}

impl Manager {
	/// Builds a manager rooted at `base`, creating the data/logs/pids/conf
	/// subdirectories and wiring the supplied engine implementations. Does not
	/// yet load the registry — call [`Manager::load_all`].
	pub fn new(
		base: impl AsRef<Path>,
		engines: Vec<Box<dyn Engine>>,
	) -> Result<Manager, ManagerError> {
		let base = base.as_ref().to_path_buf();
		let layout = Layout {
			data: base.join("data"),
			logs: base.join("logs"),
			pids: base.join("pids"),
			conf: base.join("conf"),
		};
		for dir in [&layout.data, &layout.logs, &layout.pids, &layout.conf] {
			std::fs::create_dir_all(dir).map_err(|e| ManagerError::Store(e.to_string()))?;
		}
		let store = Store::new(&base)?;
		let engines = engines.into_iter().map(|e| (e.db_type(), e)).collect();
		Ok(Manager {
			base,
			layout,
			store,
			engines,
			instances: HashMap::new(),
		})
	}

	/// Loads the persisted registry into memory, refreshing each instance's
	/// status from its engine so the in-memory view matches reality on launch.
	pub fn load_all(&mut self) -> Result<(), ManagerError> {
		self.instances.clear();
		for mut inst in self.store.read()? {
			if let Some(engine) = self.engines.get(&inst.db_type) {
				inst.status = engine.check_status(&inst);
			}
			self.instances.insert(inst.id.clone(), inst);
		}
		Ok(())
	}

	/// Every managed instance, in arbitrary order.
	pub fn list(&self) -> Vec<Instance> {
		self.instances.values().cloned().collect()
	}

	/// A copy of a single instance by id, if it exists.
	pub fn get(&self, id: &str) -> Option<Instance> {
		self.instances.get(id).cloned()
	}

	/// The installed and installable versions across every wired engine.
	pub fn versions(&self) -> Vec<Version> {
		let mut all: Vec<Version> = self.engines.values().flat_map(|e| e.versions()).collect();
		all.sort_by(|a, b| {
			a.db_type
				.as_str()
				.cmp(b.db_type.as_str())
				.then(b.major.cmp(&a.major))
		});
		all
	}

	/// Creates and initializes a new instance.
	///
	/// Rejects an empty name and an unknown engine, allocates a free port when
	/// `port` is `None` (or validates the requested one is free), lays out an
	/// isolated data directory, lets the engine initialize it, and persists the
	/// registry only after the engine succeeds.
	pub fn create(
		&mut self,
		name: &str,
		db: DbType,
		version: &str,
		port: Option<u16>,
	) -> Result<Instance, ManagerError> {
		let name = name.trim();
		if name.is_empty() {
			return Err(ManagerError::Invalid(
				"instance name must not be empty".into(),
			));
		}
		if !self.engines.contains_key(&db) {
			return Err(ManagerError::Invalid(format!(
				"unsupported engine: {}",
				db.as_str()
			)));
		}
		let port = match port {
			Some(p) if self.port_taken(p) => {
				return Err(ManagerError::Invalid(format!("port {p} is already in use")))
			}
			Some(p) => p,
			None => self.next_free_port(db)?,
		};

		let id = self.fresh_id();
		let conf_ext = conf_ext(db);
		let mut inst = Instance {
			data_dir: path_str(&self.layout.data.join(&id)),
			log_file: path_str(&self.layout.logs.join(format!("{id}.log"))),
			pid_file: path_str(&self.layout.pids.join(format!("{id}.pid"))),
			conf_file: conf_ext
				.map(|ext| path_str(&self.layout.conf.join(format!("{id}.{ext}"))))
				.unwrap_or_default(),
			id: id.clone(),
			name: name.to_string(),
			db_type: db,
			version: version.to_string(),
			port,
			user: default_user(db).to_string(),
			status: Status::Init,
			created_at: String::new(),
			last_error: String::new(),
			upgrade_version: String::new(),
			patch_update: String::new(),
		};

		let engine = self
			.engines
			.get(&db)
			.expect("engine presence checked above");
		engine.create(&mut inst)?;
		inst.status = Status::Stopped;

		self.instances.insert(id.clone(), inst.clone());
		self.persist()?;
		Ok(inst)
	}

	/// Starts the instance, recording an error status if the engine refuses.
	pub fn start(&mut self, id: &str) -> Result<(), ManagerError> {
		self.with_engine(id, |engine, inst| engine.start(inst))?;
		self.set_status(id, Status::Running);
		self.persist()
	}

	/// Stops the instance.
	pub fn stop(&mut self, id: &str) -> Result<(), ManagerError> {
		self.with_engine(id, |engine, inst| engine.stop(inst))?;
		self.set_status(id, Status::Stopped);
		self.persist()
	}

	/// Stops every running instance, best-effort. Used on shutdown.
	pub fn stop_all(&mut self) {
		let ids: Vec<String> = self.instances.keys().cloned().collect();
		for id in ids {
			let _ = self.stop(&id);
		}
	}

	/// Stops the instance and removes it and its data directory.
	pub fn delete(&mut self, id: &str) -> Result<(), ManagerError> {
		let inst = self
			.instances
			.get(id)
			.ok_or_else(|| ManagerError::NotFound(id.to_string()))?
			.clone();
		if let Some(engine) = self.engines.get(&inst.db_type) {
			let _ = engine.stop(&inst);
			engine.delete(&inst)?;
		}
		self.instances.remove(id);
		self.persist()
	}

	/// The instance's live status, recomputed from its engine.
	pub fn status(&self, id: &str) -> Result<Status, ManagerError> {
		let inst = self
			.instances
			.get(id)
			.ok_or_else(|| ManagerError::NotFound(id.to_string()))?;
		Ok(self
			.engines
			.get(&inst.db_type)
			.map(|e| e.check_status(inst))
			.unwrap_or(inst.status))
	}

	/// The next free port for `db`: the first port from the engine default
	/// upward that is neither claimed by another instance nor open on the host.
	pub fn next_free_port(&self, db: DbType) -> Result<u16, ManagerError> {
		let base = default_port(db);
		for offset in 0..PORT_SCAN_SPAN {
			let port = base.saturating_add(offset);
			if !self.port_taken(port) && port_available(port) {
				return Ok(port);
			}
		}
		Err(ManagerError::NoFreePort(db))
	}

	/// The base directory the manager is rooted at.
	pub fn base_dir(&self) -> &Path {
		&self.base
	}

	fn port_taken(&self, port: u16) -> bool {
		self.instances.values().any(|i| i.port == port)
	}

	fn with_engine<F>(&mut self, id: &str, f: F) -> Result<(), ManagerError>
	where
		F: FnOnce(&dyn Engine, &Instance) -> Result<(), EngineError>,
	{
		let inst = self
			.instances
			.get(id)
			.ok_or_else(|| ManagerError::NotFound(id.to_string()))?
			.clone();
		let engine = self.engines.get(&inst.db_type).ok_or_else(|| {
			ManagerError::Invalid(format!("unsupported engine: {}", inst.db_type.as_str()))
		})?;
		match f(engine.as_ref(), &inst) {
			Ok(()) => Ok(()),
			Err(e) => {
				let msg = e.to_string();
				if let Some(stored) = self.instances.get_mut(id) {
					stored.status = Status::Error;
					stored.last_error = msg.clone();
				}
				let _ = self.persist();
				Err(ManagerError::Engine(msg))
			}
		}
	}

	fn set_status(&mut self, id: &str, status: Status) {
		if let Some(inst) = self.instances.get_mut(id) {
			inst.status = status;
			inst.last_error = String::new();
		}
	}

	fn persist(&self) -> Result<(), ManagerError> {
		let mut all: Vec<Instance> = self.instances.values().cloned().collect();
		all.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
		self.store.write(&all)?;
		Ok(())
	}

	fn fresh_id(&self) -> String {
		loop {
			let id = random_id();
			if !self.instances.contains_key(&id) {
				return id;
			}
		}
	}
}

/// A random 12-character hex instance id.
fn random_id() -> String {
	let mut bytes = [0u8; 6];
	getrandom::getrandom(&mut bytes).expect("system RNG unavailable");
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The configuration file extension an engine uses, if it writes one.
fn conf_ext(db: DbType) -> Option<&'static str> {
	match db {
		DbType::Postgres => None,
		DbType::Mysql | DbType::Mariadb => Some("cnf"),
		DbType::Valkey | DbType::Redis => Some("conf"),
		DbType::Mongodb => Some("yml"),
	}
}

/// The default superuser/role name surfaced for an engine's connection URI.
fn default_user(db: DbType) -> &'static str {
	match db {
		DbType::Postgres => "postgres",
		DbType::Mysql | DbType::Mariadb => "root",
		DbType::Valkey | DbType::Redis | DbType::Mongodb => "",
	}
}

/// Whether a TCP port can be bound on loopback right now (i.e. nothing is
/// listening on it).
fn port_available(port: u16) -> bool {
	TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn path_str(p: &Path) -> String {
	p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
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
			Vec::new()
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
}
