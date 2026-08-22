//! Engine-agnostic domain model shared by every database engine.
//!
//! These types are the wire contract between the Rust backend and the webview:
//! their JSON shape (camelCase keys, omitted-when-empty optionals) must stay
//! byte-compatible with what the frontend consumes, so the serde attributes
//! here are load-bearing, not cosmetic.

pub mod paths;
pub mod proc;

use serde::{Deserialize, Serialize};

/// A database engine kind that KlyraDB can manage.
///
/// The serialized values are stable identifiers used as map keys, directory
/// names and engine-package lookups; they must not change without migrating
/// existing `instances.json` state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbType {
	Postgres,
	Mysql,
	Mariadb,
	Valkey,
	Redis,
	Mongodb,
}

impl DbType {
	/// Returns the stable lowercase identifier (the serialized form), usable as
	/// a directory name, map key or engine-package lookup key.
	pub fn as_str(self) -> &'static str {
		match self {
			DbType::Postgres => "postgres",
			DbType::Mysql => "mysql",
			DbType::Mariadb => "mariadb",
			DbType::Valkey => "valkey",
			DbType::Redis => "redis",
			DbType::Mongodb => "mongodb",
		}
	}

	/// Parses a stable identifier back into a [`DbType`], rejecting anything
	/// that is not one of the supported engines. Used at the bridge boundary,
	/// where the engine kind arrives as an untrusted string from the webview.
	pub fn parse(s: &str) -> Option<DbType> {
		DbType::all().into_iter().find(|db| db.as_str() == s)
	}

	/// Every supported engine kind, in display order.
	pub fn all() -> [DbType; 6] {
		[
			DbType::Postgres,
			DbType::Mysql,
			DbType::Mariadb,
			DbType::Valkey,
			DbType::Redis,
			DbType::Mongodb,
		]
	}
}

/// The lifecycle state of a managed instance.
///
/// `NeedsInstall` and `Installing` describe the engine binary's availability,
/// not the process; the rest describe the running process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
	Stopped,
	Running,
	Error,
	#[serde(rename = "initializing")]
	Init,
	NeedsInstall,
	Installing,
}

/// A managed database instance: its identity, on-disk layout and current state.
///
/// `id` is the stable handle the frontend passes back to every lifecycle call.
/// The path fields are absolute and owned exclusively by this instance, giving
/// each instance an isolated data directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
	pub id: String,
	pub name: String,
	#[serde(rename = "type")]
	pub db_type: DbType,
	pub version: String,
	pub port: u16,
	#[serde(rename = "dataDir")]
	pub data_dir: String,
	#[serde(rename = "logFile")]
	pub log_file: String,
	#[serde(rename = "pidFile")]
	pub pid_file: String,
	#[serde(rename = "confFile", default, skip_serializing_if = "String::is_empty")]
	pub conf_file: String,
	pub user: String,
	pub status: Status,
	#[serde(rename = "createdAt")]
	pub created_at: String,
	#[serde(
		rename = "lastError",
		default,
		skip_serializing_if = "String::is_empty"
	)]
	pub last_error: String,
	#[serde(
		rename = "upgradeVersion",
		default,
		skip_serializing_if = "String::is_empty"
	)]
	pub upgrade_version: String,
	#[serde(
		rename = "patchUpdate",
		default,
		skip_serializing_if = "String::is_empty"
	)]
	pub patch_update: String,
}

/// An installable/installed engine version family surfaced to the version view.
///
/// `major` is the user-facing family (e.g. `"17"` for Postgres); `installed`
/// reflects whether a matching binary was found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
	#[serde(rename = "type")]
	pub db_type: DbType,
	pub major: String,
	#[serde(rename = "binPath")]
	pub bin_path: String,
	pub installed: bool,
	pub label: String,
	#[serde(
		rename = "latestPatch",
		default,
		skip_serializing_if = "String::is_empty"
	)]
	pub latest_patch: String,
	#[serde(
		rename = "installedVersion",
		default,
		skip_serializing_if = "String::is_empty"
	)]
	pub installed_version: String,
}

/// The behaviour every engine implementation provides.
///
/// Implementations shell out to the engine's own tooling; each method acts on a
/// single instance and must leave that instance's data directory untouched on
/// failure. Defined here so the manager can treat every engine uniformly. The
/// `Send + Sync` bound lets the manager be shared across the bridge's threads.
pub trait Engine: Send + Sync {
	/// The engine kind this implementation manages.
	fn db_type(&self) -> DbType;

	/// The installed and installable version families for this engine.
	fn versions(&self) -> Vec<Version>;

	/// Initializes the instance's data directory and writes its configuration.
	fn create(&self, inst: &mut Instance) -> Result<(), EngineError>;

	/// Starts the instance's server process.
	fn start(&self, inst: &Instance) -> Result<(), EngineError>;

	/// Stops the instance's server process.
	fn stop(&self, inst: &Instance) -> Result<(), EngineError>;

	/// Removes the instance's data directory and on-disk artifacts.
	fn delete(&self, inst: &Instance) -> Result<(), EngineError>;

	/// Reports the instance's live status from its pidfile and port.
	fn check_status(&self, inst: &Instance) -> Status;
}

/// A failure raised by an [`Engine`] operation.
///
/// Carries a human-readable message already suitable for surfacing to the
/// webview; the variant marks whether the cause was the engine tooling or our
/// own I/O so callers can decide whether a retry could help.
#[derive(Debug)]
pub enum EngineError {
	/// The engine's own tooling failed (non-zero exit, refused to start, …).
	Tooling(String),
	/// A filesystem or process operation we issued failed.
	Io(String),
	/// The requested engine binary was not found on this machine.
	NotInstalled(String),
}

impl std::fmt::Display for EngineError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			EngineError::Tooling(m) => write!(f, "{m}"),
			EngineError::Io(m) => write!(f, "{m}"),
			EngineError::NotInstalled(m) => write!(f, "{m}"),
		}
	}
}

impl std::error::Error for EngineError {}

impl From<std::io::Error> for EngineError {
	fn from(e: std::io::Error) -> Self {
		EngineError::Io(e.to_string())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn db_type_serializes_to_stable_lowercase_id() {
		for db in DbType::all() {
			let json = serde_json::to_string(&db).unwrap();
			assert_eq!(json, format!("\"{}\"", db.as_str()));
		}
	}

	#[test]
	fn status_uses_the_legacy_wire_values() {
		let cases = [
			(Status::Stopped, "\"stopped\""),
			(Status::Running, "\"running\""),
			(Status::Error, "\"error\""),
			(Status::Init, "\"initializing\""),
			(Status::NeedsInstall, "\"needs_install\""),
			(Status::Installing, "\"installing\""),
		];
		for (status, want) in cases {
			assert_eq!(serde_json::to_string(&status).unwrap(), want);
		}
	}

	#[test]
	fn instance_keeps_camelcase_keys_and_omits_empty_optionals() {
		let inst = Instance {
			id: "ab12cd".into(),
			name: "dev".into(),
			db_type: DbType::Postgres,
			version: "17".into(),
			port: 5432,
			data_dir: "/data/ab12cd".into(),
			log_file: "/logs/ab12cd.log".into(),
			pid_file: "/pids/ab12cd.pid".into(),
			conf_file: String::new(),
			user: "postgres".into(),
			status: Status::Stopped,
			created_at: "2026-06-29T00:00:00Z".into(),
			last_error: String::new(),
			upgrade_version: String::new(),
			patch_update: String::new(),
		};
		let v: serde_json::Value = serde_json::to_value(&inst).unwrap();
		let obj = v.as_object().unwrap();
		assert_eq!(obj["type"], "postgres");
		assert_eq!(obj["dataDir"], "/data/ab12cd");
		assert_eq!(obj["createdAt"], "2026-06-29T00:00:00Z");
		// omitempty fields drop out entirely when empty.
		assert!(!obj.contains_key("confFile"));
		assert!(!obj.contains_key("lastError"));
		assert!(!obj.contains_key("upgradeVersion"));
		assert!(!obj.contains_key("patchUpdate"));
	}

	#[test]
	fn parse_round_trips_every_kind_and_rejects_unknown() {
		for db in DbType::all() {
			assert_eq!(DbType::parse(db.as_str()), Some(db));
		}
		assert_eq!(DbType::parse("oracle"), None);
		assert_eq!(DbType::parse(""), None);
	}

	#[test]
	fn engine_error_displays_its_message_and_wraps_io() {
		assert_eq!(EngineError::Tooling("x".into()).to_string(), "x");
		assert_eq!(EngineError::NotInstalled("y".into()).to_string(), "y");
		let io = std::io::Error::other("disk");
		assert_eq!(EngineError::from(io).to_string(), "disk");
	}

	#[test]
	fn instance_round_trips_through_json() {
		let json = r#"{
			"id":"ab12cd","name":"dev","type":"valkey","version":"8","port":6379,
			"dataDir":"/d","logFile":"/l","pidFile":"/p","user":"",
			"status":"running","createdAt":"2026-06-29T00:00:00Z"
		}"#;
		let inst: Instance = serde_json::from_str(json).unwrap();
		assert_eq!(inst.db_type, DbType::Valkey);
		assert_eq!(inst.port, 6379);
		assert_eq!(inst.status, Status::Running);
		assert!(inst.conf_file.is_empty());
	}
}
