//! Every interface text has its French translation, and the catalog has no stale entry.
//! Texts are found where the code marks them: tr("…") in Rust, t("…") and the key
//! label tables in JavaScript, data-i18n attributes in HTML.

use regex::Regex;
use std::collections::BTreeSet;
use std::path::Path;

const STRING: &str = r#""((?:[^"\\]|\\.)*)""#;

fn read(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn capture(found: &mut BTreeSet<String>, text: &str, pattern: &str) {
    for c in Regex::new(pattern).unwrap().captures_iter(text) {
        found.insert(c[1].replace("\\\"", "\"").replace("\\\\", "\\"));
    }
}

/// The string values of `const NAME = { … };` in JavaScript.
fn js_table(found: &mut BTreeSet<String>, js: &str, name: &str) {
    let start = js.find(&format!("const {name} = {{")).expect(name);
    let block = &js[start..start + js[start..].find("};").unwrap()];
    capture(found, block, &format!(r":\s*{STRING}"));
}

fn used_texts() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for file in [
        "src-tauri/src/tray.rs",
        "src-tauri/src/settings.rs",
        "src-tauri/src/discord.rs",
        "src-tauri/src/host.rs",
    ] {
        let rust = read(file);
        capture(&mut found, &rust, &format!(r"\btr\(\s*{STRING}"));
        // Reasons shown in the tray and the window, translated where shown.
        capture(&mut found, &rust, &format!(r"\bneeds\(\s*{STRING}"));
        // Browser pages of the authorization fallback.
        capture(
            &mut found,
            &rust,
            &format!(r"\(\s*&mut pages\.\w+,\s*{STRING}"),
        );
    }
    let js = read("ui/settings.js");
    capture(&mut found, &js, &format!(r"\bt\(\s*{STRING}"));
    js_table(&mut found, &js, "NAMED_KEYS");
    js_table(&mut found, &js, "META_KEYS");
    let html = read("ui/settings.html");
    capture(&mut found, &html, r#"data-i18n(?:-placeholder)?="([^"]*)""#);
    // Language names are shown as they are, except "Automatic".
    found.insert("Automatic".into());
    found
}

#[test]
fn french_catalog_is_complete_and_clean() {
    let catalog: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&read("crates/core/i18n/fr.json")).unwrap();
    let used = used_texts();
    let missing: Vec<&String> = used.iter().filter(|t| !catalog.contains_key(*t)).collect();
    let stale: Vec<&String> = catalog.keys().filter(|k| !used.contains(*k)).collect();
    assert!(missing.is_empty(), "missing in fr.json: {missing:#?}");
    assert!(stale.is_empty(), "unused in fr.json: {stale:#?}");
}
