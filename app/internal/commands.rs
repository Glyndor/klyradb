//! Tauri command surface — the webview↔backend bridge.
//!
//! Every function here is a trust boundary: arguments arrive from web content
//! and must be validated before they reach the manager or the host. Engine
//! kinds are parsed against the known set (never used to build a path or
//! command), and ids are only ever looked up, never interpolated into shell
//! invocations. Long-running installs stream progress as `install:progress:<id>`
//! events instead of blocking the call.

use std::collections::BTreeMap;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, State};

use klyradb::engine::{DbType, Instance, Status, Version};
use klyradb::i18n::{Catalog, Lang};
use klyradb::install;
use klyradb::manager::Manager;

/// Shared application state behind the bridge.
pub struct AppState {
	/// The instance manager. Guarded by a mutex because commands run on a pool.
	pub manager: Mutex<Manager>,
	/// The active locale and the catalogue it was resolved from.
	pub locale: Mutex<LocaleState>,
}

/// The localization state held across the session.
pub struct LocaleState {
	/// The embedded catalogue of every shipped locale.
	pub catalog: Catalog,
	/// The currently active resolved locale.
	pub lang: Lang,
}

/// Every installed/installable engine version.
#[tauri::command]
pub fn list_versions(state: State<'_, AppState>) -> Vec<Version> {
	state.manager.lock().unwrap().versions()
}

/// Every managed instance.
#[tauri::command]
pub fn list_instances(state: State<'_, AppState>) -> Vec<Instance> {
	state.manager.lock().unwrap().list()
}

/// Creates a new instance of `db_type`/`version`, optionally on a fixed `port`.
#[tauri::command]
pub fn create_instance(
	state: State<'_, AppState>,
	name: String,
	db_type: String,
	version: String,
	port: Option<u16>,
) -> Result<Instance, String> {
	let db = DbType::parse(&db_type).ok_or_else(|| format!("unknown engine: {db_type}"))?;
	state
		.manager
		.lock()
		.unwrap()
		.create(&name, db, &version, port)
		.map_err(|e| e.to_string())
}

/// Starts an instance.
#[tauri::command]
pub fn start_instance(state: State<'_, AppState>, id: String) -> Result<(), String> {
	state
		.manager
		.lock()
		.unwrap()
		.start(&id)
		.map_err(|e| e.to_string())
}

/// Stops an instance.
#[tauri::command]
pub fn stop_instance(state: State<'_, AppState>, id: String) -> Result<(), String> {
	state
		.manager
		.lock()
		.unwrap()
		.stop(&id)
		.map_err(|e| e.to_string())
}

/// Stops and removes an instance, including its data directory.
#[tauri::command]
pub fn delete_instance(state: State<'_, AppState>, id: String) -> Result<(), String> {
	state
		.manager
		.lock()
		.unwrap()
		.delete(&id)
		.map_err(|e| e.to_string())
}

/// The live status of an instance.
#[tauri::command]
pub fn instance_status(state: State<'_, AppState>, id: String) -> Result<Status, String> {
	state
		.manager
		.lock()
		.unwrap()
		.status(&id)
		.map_err(|e| e.to_string())
}

/// The next free port KlyraDB would suggest for `db_type`.
#[tauri::command]
pub fn suggest_port(state: State<'_, AppState>, db_type: String) -> Result<u16, String> {
	let db = DbType::parse(&db_type).ok_or_else(|| format!("unknown engine: {db_type}"))?;
	state
		.manager
		.lock()
		.unwrap()
		.next_free_port(db)
		.map_err(|e| e.to_string())
}

/// Installs the engine binary for an instance, streaming progress as
/// `install:progress:<id>` events. The manager runs the Linux-specific
/// "blocked" check first (MongoDB on Linux), then drives the install command.
#[tauri::command]
pub fn install_binary(
	app: AppHandle,
	state: State<'_, AppState>,
	id: String,
) -> Result<(), String> {
	let event = format!("install:progress:{id}");
	let mut mgr = state.manager.lock().unwrap();
	mgr.install(&id, |line| {
		let _ = app.emit(&event, line.to_string());
	})
	.map_err(|e| e.to_string())
}

/// Re-runs the package install for an instance to pick up a patch release, then
/// restarts it. Streams progress like [`install_binary`].
#[tauri::command]
pub fn upgrade_patch(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
	let event = format!("install:progress:{id}");
	let mut mgr = state.manager.lock().unwrap();
	mgr.upgrade_patch(&id, |line| {
		let _ = app.emit(&event, line.to_string());
	})
	.map_err(|e| e.to_string())
}

/// Switches the active locale and returns the full string table for it.
#[tauri::command]
pub fn set_locale(state: State<'_, AppState>, code: String) -> BTreeMap<String, String> {
	let mut locale = state.locale.lock().unwrap();
	locale.lang = locale.catalog.for_code(&code);
	locale.lang.strings().clone()
}

/// The full string table for the active locale.
#[tauri::command]
pub fn strings(state: State<'_, AppState>) -> BTreeMap<String, String> {
	state.locale.lock().unwrap().lang.strings().clone()
}

/// The active locale code.
#[tauri::command]
pub fn locale(state: State<'_, AppState>) -> String {
	state.locale.lock().unwrap().lang.code.clone()
}

/// The active locale's text direction (`ltr`/`rtl`).
#[tauri::command]
pub fn direction(state: State<'_, AppState>) -> String {
	state.locale.lock().unwrap().lang.dir.clone()
}

/// Every shipped locale code.
#[tauri::command]
pub fn available_locales(state: State<'_, AppState>) -> Vec<String> {
	state.locale.lock().unwrap().catalog.available()
}
