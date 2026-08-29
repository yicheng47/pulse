pub mod library;
mod playback;
mod preferences;
mod queue;
mod settings;
mod updater;

pub(crate) use library::*;
pub(crate) use playback::*;
pub(crate) use preferences::*;
pub(crate) use queue::*;
pub(crate) use settings::*;
pub(crate) use updater::*;

#[cfg(test)]
mod boundary_tests {
    use std::{fs, path::Path};

    #[test]
    fn backend_avoids_ui_framework() {
        let backend = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backend");
        // Split the forbidden name so the gate does not flag its own source file.
        let forbidden = ["gp", "ui"].concat();
        let mut offenders = Vec::new();
        scan_directory(&backend, &backend, &forbidden, &mut offenders);

        assert!(
            offenders.is_empty(),
            "backend files import the UI framework: {}",
            offenders.join(", ")
        );
    }

    #[test]
    fn detects_forbidden_imports_after_string_literals() {
        let forbidden = ["gp", "ui"].concat();
        assert!(source_contains_forbidden(
            &format!("use {forbidden}::Context;"),
            &forbidden
        ));
        assert!(source_contains_forbidden(
            &format!("let url = \"https://example.com\"; use {forbidden}::Context;"),
            &forbidden
        ));
        assert!(!source_contains_forbidden(
            &format!("// use {forbidden}::Context;"),
            &forbidden
        ));
    }

    fn scan_directory(root: &Path, directory: &Path, forbidden: &str, offenders: &mut Vec<String>) {
        for entry in fs::read_dir(directory).expect("failed to read backend directory") {
            let path = entry.expect("failed to read backend entry").path();
            if path.is_dir() {
                scan_directory(root, &path, forbidden, offenders);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = fs::read_to_string(&path).expect("failed to read backend source");
                if source_contains_forbidden(&source, forbidden) {
                    offenders.push(
                        path.strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string(),
                    );
                }
            }
        }
    }

    fn source_contains_forbidden(source: &str, forbidden: &str) -> bool {
        source.lines().any(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//")
                && !trimmed.starts_with("/*")
                && !trimmed.starts_with('*')
                && line.contains(forbidden)
        })
    }
}
