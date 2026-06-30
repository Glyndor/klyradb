//! KlyraDB desktop application entry point.
//!
//! Wires the engine implementations into the manager, loads the locale
//! catalogue, and hands both to the Tauri runtime as shared state behind the
//! command bridge.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

mod commands;
use commands::{AppState, LocaleState};
use klyradb::engine::paths::base_dir;
use klyradb::engine::Engine;
use klyradb::i18n::Catalog;
use klyradb::manager::Manager;
use klyradb::{mariadb, mongodb, mysql, pg, redis};

/// Builds the manager with every supported engine wired in and the persisted
/// registry loaded.
fn build_manager() -> Manager {
	let engines: Vec<Box<dyn Engine>> = vec![
		Box::new(pg::PgEngine::new()),
		Box::new(mysql::MysqlEngine::new()),
		Box::new(mariadb::MariadbEngine::new()),
		Box::new(redis::RespEngine::valkey()),
		Box::new(redis::RespEngine::redis()),
		Box::new(mongodb::MongoEngine::new()),
	];
	let mut manager = Manager::new(base_dir(), engines).expect("failed to initialize the manager");
	let _ = manager.load_all();
	manager
}

fn main() {
	let catalog = Catalog::load();
	let lang = catalog.detect();
	let state = AppState {
		manager: Mutex::new(build_manager()),
		locale: Mutex::new(LocaleState { catalog, lang }),
	};

	tauri::Builder::default()
		.manage(state)
		.invoke_handler(tauri::generate_handler![
			commands::list_versions,
			commands::list_instances,
			commands::create_instance,
			commands::start_instance,
			commands::stop_instance,
			commands::delete_instance,
			commands::instance_status,
			commands::suggest_port,
			commands::install_binary,
			commands::upgrade_patch,
			commands::set_locale,
			commands::strings,
			commands::locale,
			commands::direction,
			commands::available_locales,
		])
		.run(tauri::generate_context!())
		.expect("error while running the KlyraDB application");
}
