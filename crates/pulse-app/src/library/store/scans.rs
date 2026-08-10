use rusqlite::{Connection, Transaction, params};

use super::super::{LibraryError, ScanHistoryEntry, ScanOutcome, StorageRootId};
use super::{roots, usize_to_i64};

pub struct CompletedScan {
    pub finished_at_ms: i64,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unsupported: usize,
    pub errors: usize,
    pub removals_suppressed: bool,
    pub outcome: ScanOutcome,
}

pub fn begin(
    conn: &Connection,
    scan_session_id: &str,
    storage_root_id: StorageRootId,
    started_at_ms: i64,
) -> Result<i64, LibraryError> {
    conn.query_row(
        "INSERT INTO scan_history (storage_root_id, scan_session_id, started_at_ms)
         VALUES (?1, ?2, ?3)
         RETURNING id",
        params![storage_root_id, scan_session_id, started_at_ms],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub fn cancel(conn: &Connection, scan_id: i64) -> Result<(), LibraryError> {
    conn.execute(
        "DELETE FROM scan_history
         WHERE id = ?1 AND finished_at_ms IS NULL AND outcome IS NULL",
        [scan_id],
    )?;
    Ok(())
}

pub fn finish_offline(
    transaction: &Transaction<'_>,
    scan_id: i64,
    storage_root_id: StorageRootId,
    finished_at_ms: i64,
    error_message: &str,
) -> Result<(), LibraryError> {
    roots::record_scan_outcome(transaction, storage_root_id, finished_at_ms, Some(false))?;
    transaction.execute(
        "UPDATE scan_history
         SET finished_at_ms = ?2, error_count = 1, outcome = 'offline',
             error_message = ?3
         WHERE id = ?1",
        params![scan_id, finished_at_ms, error_message],
    )?;
    Ok(())
}

pub fn finish_failed(
    transaction: &Transaction<'_>,
    scan_id: i64,
    storage_root_id: StorageRootId,
    finished_at_ms: i64,
    error_message: &str,
) -> Result<(), LibraryError> {
    roots::record_scan_outcome(transaction, storage_root_id, finished_at_ms, None)?;
    transaction.execute(
        "UPDATE scan_history
         SET finished_at_ms = ?2, error_count = 1, outcome = 'failed',
             error_message = ?3
         WHERE id = ?1",
        params![scan_id, finished_at_ms, error_message],
    )?;
    Ok(())
}

pub fn finish_completed_scan(
    transaction: &Transaction<'_>,
    scan_id: i64,
    storage_root_id: StorageRootId,
    completed: &CompletedScan,
) -> Result<(), LibraryError> {
    roots::record_scan_outcome(
        transaction,
        storage_root_id,
        completed.finished_at_ms,
        Some(true),
    )?;
    transaction.execute(
        "UPDATE scan_history
         SET finished_at_ms = ?2, added_count = ?3, updated_count = ?4,
             removed_count = ?5, unsupported_count = ?6, error_count = ?7,
             removals_suppressed = ?8, outcome = ?9
         WHERE id = ?1",
        params![
            scan_id,
            completed.finished_at_ms,
            usize_to_i64(completed.added, "added count")?,
            usize_to_i64(completed.updated, "updated count")?,
            usize_to_i64(completed.removed, "removed count")?,
            usize_to_i64(completed.unsupported, "unsupported count")?,
            usize_to_i64(completed.errors, "error count")?,
            completed.removals_suppressed,
            completed.outcome.as_db_str(),
        ],
    )?;
    Ok(())
}

pub fn recent(
    conn: &Connection,
    storage_root_id: StorageRootId,
    limit: usize,
) -> Result<Vec<ScanHistoryEntry>, LibraryError> {
    let limit =
        i64::try_from(limit).map_err(|_| LibraryError::IntegerOutOfRange("scan history limit"))?;
    let mut statement = conn.prepare(
        "SELECT id, storage_root_id, started_at_ms, finished_at_ms,
                added_count, updated_count, removed_count, unsupported_count,
                error_count, removals_suppressed, outcome, error_message
         FROM scan_history
         WHERE storage_root_id = ?1
         ORDER BY started_at_ms DESC, id DESC
         LIMIT ?2",
    )?;
    let scans = statement
        .query_map(params![storage_root_id, limit], scan_history_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(scans)
}

/// Closes scans another session left unfinished — the app crashed or was
/// killed mid-scan — so they surface as failed instead of forever-running.
pub fn recover_interrupted(
    conn: &Connection,
    recovered_at_ms: i64,
    scan_session_id: &str,
) -> Result<(), LibraryError> {
    conn.execute(
        "UPDATE scan_history
         SET finished_at_ms = ?1,
             error_count = CASE WHEN error_count = 0 THEN 1 ELSE error_count END,
             outcome = 'failed',
             error_message = 'Scan interrupted before completion'
         WHERE finished_at_ms IS NULL
           AND outcome IS NULL
           AND scan_session_id <> ?2",
        params![recovered_at_ms, scan_session_id],
    )?;
    Ok(())
}

pub fn delete_for_root(
    conn: &Connection,
    storage_root_id: StorageRootId,
) -> Result<(), LibraryError> {
    conn.execute(
        "DELETE FROM scan_history WHERE storage_root_id = ?1",
        [storage_root_id],
    )?;
    Ok(())
}

fn scan_history_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanHistoryEntry> {
    let outcome = row
        .get::<_, Option<String>>(10)?
        .map(|outcome| {
            ScanOutcome::from_db_str(&outcome).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    format!("unknown scan outcome {outcome}").into(),
                )
            })
        })
        .transpose()?;
    Ok(ScanHistoryEntry {
        id: row.get(0)?,
        storage_root_id: row.get(1)?,
        started_at_ms: row.get(2)?,
        finished_at_ms: row.get(3)?,
        added_count: row.get::<_, i64>(4)? as u64,
        updated_count: row.get::<_, i64>(5)? as u64,
        removed_count: row.get::<_, i64>(6)? as u64,
        unsupported_count: row.get::<_, i64>(7)? as u64,
        error_count: row.get::<_, i64>(8)? as u64,
        removals_suppressed: row.get(9)?,
        outcome,
        error_message: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::library::{
        LibraryStore, ScanOutcome,
        store::testing::{insert_track, test_file, test_metadata},
    };

    #[test]
    fn marks_an_offline_root_without_removing_its_tracks() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root = store.add_storage_root(temp.path(), "Music").unwrap();
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "track.wav", 1, 10),
            &test_metadata("Track", "Artist", Some("Album"), None),
        );
        let scan_id = store.begin_scan(root.id, 100).unwrap();

        store
            .finish_offline_scan(scan_id, root.id, 200, "not mounted")
            .unwrap();

        assert_eq!(store.tracks_for_root(root.id).unwrap().len(), 1);
        let stored_root = store.storage_root(root.id).unwrap().unwrap();
        assert!(!stored_root.is_reachable);
        assert_eq!(stored_root.last_scan_at_ms, Some(200));
        let history = store.recent_scans(root.id, 1).unwrap();
        assert_eq!(history[0].outcome, Some(ScanOutcome::Offline));
        assert_eq!(history[0].error_count, 1);
    }

    #[test]
    fn reopens_persistent_data_and_recovers_an_interrupted_other_session() {
        let temp = tempdir().unwrap();
        let music = temp.path().join("music");
        fs::create_dir(&music).unwrap();
        let database_path = temp.path().join("library.sqlite");
        let mut first = LibraryStore::from_connection(
            Connection::open(&database_path).unwrap(),
            "first-session".to_string(),
            |_| {},
        )
        .unwrap();
        let root = first.add_storage_root(&music, "Music").unwrap();
        let root_id = root.id;
        let scan_id = first.begin_scan(root.id, 100).unwrap();
        drop(first);

        let same_session = LibraryStore::from_connection(
            Connection::open(&database_path).unwrap(),
            "first-session".to_string(),
            |_| {},
        )
        .unwrap();
        assert_eq!(
            same_session.recent_scans(root_id, 1).unwrap()[0].outcome,
            None
        );
        drop(same_session);

        let reopened = LibraryStore::from_connection(
            Connection::open(&database_path).unwrap(),
            "second-session".to_string(),
            |_| {},
        )
        .unwrap();
        assert_eq!(reopened.storage_roots().unwrap(), vec![root]);
        let history = reopened.recent_scans(root_id, 1).unwrap();
        assert_eq!(history[0].id, scan_id);
        assert_eq!(history[0].outcome, Some(ScanOutcome::Failed));
        assert!(history[0].finished_at_ms.is_some());
        assert_eq!(
            history[0].error_message.as_deref(),
            Some("Scan interrupted before completion")
        );
    }
}
