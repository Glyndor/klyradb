//! Localization catalogue.
//!
//! Every locale file under `locales/` is embedded into the binary at compile
//! time, so the app ships its translations with no runtime file lookups. The
//! English catalogue is the source of truth: any locale that is missing a key
//! falls back to the English string, so the UI is never blank or crashing on a
//! gap. Brand and product names are kept verbatim in the locale files, never
//! translated here.

use std::collections::BTreeMap;

use include_dir::{include_dir, Dir};

/// The embedded locale files (`<code>.json`, flat key → string maps).
static LOCALES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/locales");

/// The locale used as the source of truth and the fallback for missing keys.
pub const FALLBACK: &str = "en";

/// Locale codes that render right-to-left.
const RTL: [&str; 4] = ["ar", "he", "fa", "ur"];

/// A loaded locale: its code, text direction and resolved strings.
pub struct Lang {
	/// The resolved locale code (e.g. `"es"`, `"pt-br"`).
	pub code: String,
	/// `"rtl"` for right-to-left scripts, otherwise `"ltr"`.
	pub dir: String,
	strings: BTreeMap<String, String>,
}

impl Lang {
	/// The translation for `key`, or the key itself if no string is defined
	/// even in the English fallback (so a missing key is visible, not blank).
	pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
		self.strings.get(key).map(String::as_str).unwrap_or(key)
	}

	/// Every resolved key/value pair, English-completed. This is the payload
	/// handed to the webview in one shot when the locale changes.
	pub fn strings(&self) -> &BTreeMap<String, String> {
		&self.strings
	}
}

/// The embedded localization catalogue.
pub struct Catalog {
	locales: BTreeMap<String, BTreeMap<String, String>>,
}

impl Catalog {
	/// Parses every embedded locale file into memory.
	///
	/// # Panics
	/// Panics if an embedded locale file is not valid flat JSON — that is a
	/// build-time authoring error in this repo, never a runtime input, so it
	/// must fail loudly rather than silently ship a broken catalogue.
	pub fn load() -> Catalog {
		let mut locales = BTreeMap::new();
		for file in LOCALES.files() {
			let Some(stem) = file.path().file_stem().and_then(|s| s.to_str()) else {
				continue;
			};
			if file.path().extension().and_then(|e| e.to_str()) != Some("json") {
				continue;
			}
			let body = file.contents_utf8().expect("locale file is not UTF-8");
			let map: BTreeMap<String, String> = serde_json::from_str(body)
				.unwrap_or_else(|e| panic!("locale {stem} is not flat JSON: {e}"));
			locales.insert(normalize(stem), map);
		}
		Catalog { locales }
	}

	/// Every available locale code, sorted.
	pub fn available(&self) -> Vec<String> {
		self.locales.keys().cloned().collect()
	}

	/// Whether `code` (after normalization) is shipped.
	pub fn has(&self, code: &str) -> bool {
		self.locales.contains_key(&normalize(code))
	}

	/// Resolves `code` to a [`Lang`], completing it against English.
	///
	/// `code` is normalized (lowercased, `_` → `-`); an exact match wins, else
	/// the bare language subtag is tried (`pt-pt` → `pt`), else the English
	/// fallback. The returned strings always cover every English key.
	pub fn for_code(&self, code: &str) -> Lang {
		let resolved = self.resolve(code);
		let mut strings = self.locales.get(FALLBACK).cloned().unwrap_or_default();
		if resolved != FALLBACK {
			if let Some(overlay) = self.locales.get(&resolved) {
				for (k, v) in overlay {
					strings.insert(k.clone(), v.clone());
				}
			}
		}
		Lang {
			dir: direction(&resolved).to_string(),
			code: resolved,
			strings,
		}
	}

	/// Detects the best locale from the environment, falling back to English.
	///
	/// Reads `LC_ALL`, then `LC_MESSAGES`, then `LANG` (POSIX precedence) and
	/// matches the first that resolves to a shipped locale.
	pub fn detect(&self) -> Lang {
		for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
			if let Ok(val) = std::env::var(var) {
				if val.is_empty() {
					continue;
				}
				let code = normalize(val.split('.').next().unwrap_or(&val));
				if self.has(&code) || self.has(lang_subtag(&code)) {
					return self.for_code(&code);
				}
			}
		}
		self.for_code(FALLBACK)
	}

	fn resolve(&self, code: &str) -> String {
		let norm = normalize(code);
		if self.locales.contains_key(&norm) {
			return norm;
		}
		let subtag = lang_subtag(&norm).to_string();
		if self.locales.contains_key(&subtag) {
			return subtag;
		}
		FALLBACK.to_string()
	}
}

/// Normalizes a locale code: lowercase and `_` → `-` (`pt_BR` → `pt-br`).
fn normalize(code: &str) -> String {
	code.trim().to_ascii_lowercase().replace('_', "-")
}

/// The bare language subtag of a normalized code (`pt-br` → `pt`).
fn lang_subtag(code: &str) -> &str {
	code.split('-').next().unwrap_or(code)
}

/// The text direction for a resolved locale code.
fn direction(code: &str) -> &'static str {
	if RTL.contains(&lang_subtag(code)) {
		"rtl"
	} else {
		"ltr"
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn english_is_shipped_and_is_the_fallback() {
		let cat = Catalog::load();
		assert!(cat.has(FALLBACK));
		assert!(cat.available().contains(&"en".to_string()));
	}

	#[test]
	fn missing_key_falls_back_to_the_key_itself_never_blank() {
		let cat = Catalog::load();
		let en = cat.for_code("en");
		assert_eq!(
			en.t("definitely.not.a.real.key"),
			"definitely.not.a.real.key"
		);
	}

	#[test]
	fn a_partial_locale_is_completed_against_english() {
		let cat = Catalog::load();
		let en = cat.for_code("en");
		let es = cat.for_code("es");
		// Spanish must expose every English key, falling back where it lacks one.
		for key in en.strings().keys() {
			assert!(es.strings().contains_key(key), "es missing key {key}");
		}
	}

	#[test]
	fn unknown_locale_resolves_to_english() {
		let cat = Catalog::load();
		assert_eq!(cat.for_code("xx").code, "en");
	}

	#[test]
	fn region_code_falls_back_to_language_subtag() {
		let cat = Catalog::load();
		// pt-pt is not shipped, but pt is — so it resolves to pt, not en.
		if cat.has("pt") {
			assert_eq!(cat.for_code("pt-pt").code, "pt");
		}
	}

	#[test]
	fn rtl_locales_report_rtl_direction() {
		let cat = Catalog::load();
		if cat.has("ar") {
			assert_eq!(cat.for_code("ar").dir, "rtl");
		}
		assert_eq!(cat.for_code("en").dir, "ltr");
	}
}
