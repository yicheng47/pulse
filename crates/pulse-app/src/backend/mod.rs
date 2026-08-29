mod model;
pub(crate) mod ops;
mod playback;
mod preferences;
mod queue;
mod repo;
pub(crate) mod scan;
mod settings;
mod updater;

pub use model::*;
pub(crate) use playback::*;
pub(crate) use preferences::*;
pub(crate) use queue::*;
pub use repo::{BackfillProgress, LibraryStore};
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
    fn library_sql_and_database_driver_stay_in_repo() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let repo_root = source_root.join("backend/repo");
        let driver = ["rusq", "lite"].concat();
        let sql_prefixes = ["SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE"]
            .map(|keyword| ["\"", keyword, " "].concat());
        let album_artist_fragment = ["NULLIF", "(trim(album_artist)"].concat();
        let track_artist_fragment = [
            "COALESCE(NULLIF",
            "(trim(artist), ''), ",
            "'Unknown Artist')",
        ]
        .concat();
        let mut sources = Vec::new();
        collect_rust_sources(&source_root, &mut sources);

        let mut offenders = Vec::new();
        let mut fragment_files = Vec::new();
        let mut fragment_occurrences = 0;
        let mut track_fragment_files = Vec::new();
        let mut track_fragment_occurrences = 0;
        for path in sources {
            let source = fs::read_to_string(&path).expect("failed to read app source");
            let occurrences = source.matches(&album_artist_fragment).count();
            if occurrences > 0 {
                fragment_occurrences += occurrences;
                fragment_files.push(path.clone());
            }
            let track_occurrences = source.matches(&track_artist_fragment).count();
            if track_occurrences > 0 {
                track_fragment_occurrences += track_occurrences;
                track_fragment_files.push(path.clone());
            }
            if !path.starts_with(&repo_root)
                && source_contains_library_persistence(&source, &driver, &sql_prefixes)
            {
                offenders.push(
                    path.strip_prefix(&source_root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                );
            }
        }

        assert!(
            offenders.is_empty(),
            "library persistence escaped backend/repo: {}",
            offenders.join(", ")
        );
        assert_eq!(
            fragment_occurrences,
            1,
            "effective album-artist SQL must be defined in exactly one file: {}",
            fragment_files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(
            fragment_files[0].starts_with(&repo_root),
            "effective album-artist SQL must stay under backend/repo: {}",
            fragment_files[0].display()
        );
        assert_eq!(
            track_fragment_occurrences,
            1,
            "effective track-artist SQL must be defined in exactly one file: {}",
            track_fragment_files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(
            track_fragment_files[0].starts_with(&repo_root),
            "effective track-artist SQL must stay under backend/repo: {}",
            track_fragment_files[0].display()
        );
    }

    #[test]
    fn library_boundary_gate_ignores_comments_but_detects_code() {
        let driver = ["rusq", "lite"].concat();
        let sql_prefixes = ["SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE"]
            .map(|keyword| ["\"", keyword, " "].concat());
        let select = ["let sql = ", "\"", "SELECT", " tracks\";"].concat();

        assert!(source_contains_library_persistence(
            &format!("use {driver}::Connection;"),
            &driver,
            &sql_prefixes
        ));
        assert!(source_contains_library_persistence(
            &select,
            &driver,
            &sql_prefixes
        ));
        assert!(!source_contains_library_persistence(
            &format!("// use {driver}::Connection;\n/// {select}"),
            &driver,
            &sql_prefixes
        ));
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

    fn collect_rust_sources(directory: &Path, sources: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(directory).expect("failed to read source directory") {
            let path = entry.expect("failed to read source entry").path();
            if path.is_dir() {
                collect_rust_sources(&path, sources);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }

    fn source_contains_forbidden(source: &str, forbidden: &str) -> bool {
        code_lines(source).any(|line| line.contains(forbidden))
    }

    fn source_contains_library_persistence(
        source: &str,
        driver: &str,
        sql_prefixes: &[String],
    ) -> bool {
        code_lines(source).any(|line| {
            line.contains(driver) || sql_prefixes.iter().any(|prefix| line.contains(prefix))
        })
    }

    fn code_lines(source: &str) -> impl Iterator<Item = &str> {
        source.lines().filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && !trimmed.starts_with("/*") && !trimmed.starts_with('*')
        })
    }
}
