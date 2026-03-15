use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("nestum_{label}_{nanos}"))
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, contents).expect("fixture file should be writable");
}

#[test]
fn rustdoc_hides_backing_names_from_downstream_surface() {
    let crate_dir = unique_temp_dir("rustdoc_surface");
    let target_dir = crate_dir.join("target");
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    write_file(
        &crate_dir.join("Cargo.toml"),
        &format!(
            r#"[package]
name = "nestum_doc_consumer"
version = "0.1.0"
edition = "2024"

[dependencies]
nestum = {{ path = "{}" }}
"#,
            manifest_dir.display()
        ),
    );

    write_file(
        &crate_dir.join("src/lib.rs"),
        r#"use nestum::nestum;

#[nestum]
pub enum DocumentEvent {
    Created,
    Deleted,
}

#[nestum]
pub enum Event {
    Document(DocumentEvent),
    Health,
}

pub fn build() -> Event::Enum {
    Event::Document::Created
}
"#,
    );

    let output = Command::new("cargo")
        .arg("doc")
        .arg("--quiet")
        .arg("--no-deps")
        .arg("--target-dir")
        .arg(&target_dir)
        .current_dir(&crate_dir)
        .output()
        .expect("cargo doc should run");

    if !output.status.success() {
        panic!(
            "cargo doc failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let crate_index = fs::read_to_string(target_dir.join("doc/nestum_doc_consumer/index.html"))
        .expect("crate index exists");
    let all_items = fs::read_to_string(target_dir.join("doc/nestum_doc_consumer/all.html"))
        .expect("all items page exists");
    let event_page =
        fs::read_to_string(target_dir.join("doc/nestum_doc_consumer/Event/index.html"))
            .expect("event module page exists");

    assert!(crate_index.contains("DocumentEvent"));
    assert!(!crate_index.contains("__NestumEvent"));
    assert!(crate_index.contains("Event"));
    assert!(all_items.contains("Event::Enum"));
    assert!(all_items.contains("Event::Document::Created"));
    assert!(all_items.contains("Event::Health"));
    assert!(!all_items.contains("__NestumEvent"));
    assert!(!all_items.contains("__NestumDocumentEvent"));
    assert!(!event_page.contains("pub use self::__NestumEvent::Health;"));
    assert!(event_page.contains("constant.Health.html"));

    let _ = fs::remove_dir_all(&crate_dir);
}
