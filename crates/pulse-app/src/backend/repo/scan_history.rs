use rusqlite::params;

use super::super::{LibraryError, ScanHistoryEntry, ScanOutcome, StorageRootId};
use super::{LibraryStore, LibraryTransaction, artists, select_list, storage_roots, usize_to_i64};

const COLUMNS: &[&str] = &[
    "id",
    "storage_root_id",
    "scan_session_id",
    "started_at_ms",
    "finished_at_ms",
    "added_count",
    "updated_count",
    "removed_count",
    "unsupported_count",
    "error_count",
    "removals_suppressed",
    "outcome",
    "error_message",
];

struct ScanHistoryRow {
    id: i64,
    storage_root_id: StorageRootId,
    _scan_session_id: String,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    added_count: i64,
    updated_count: i64,
    removed_count: i64,
    unsupported_count: i64,
    error_count: i64,
    removals_suppressed: bool,
    outcome: Option<String>,
    error_message: Option<String>,
}

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
    store: &mut LibraryStore,
    storage_root_id: StorageRootId,
    started_at_ms: i64,
) -> Result<i64, LibraryError> {
    let conn = &store.connection;
    conn.query_row(
        "INSERT INTO scan_history (storage_root_id, scan_session_id, started_at_ms)
         VALUES (?1, ?2, ?3)
         RETURNING id",
        params![storage_root_id, store.scan_session_id, started_at_ms],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub fn cancel(transaction: &LibraryTransaction<'_>, scan_id: i64) -> Result<(), LibraryError> {
    transaction.inner.execute(
        "DELETE FROM scan_history
         WHERE id = ?1 AND finished_at_ms IS NULL AND outcome IS NULL",
        [scan_id],
    )?;
    Ok(())
}

pub fn cancel_and_refresh(store: &mut LibraryStore, scan_id: i64) -> Result<(), LibraryError> {
    let refreshed_at_ms = super::super::scan::system_time_ms(std::time::SystemTime::now())?;
    let transaction = store.transaction()?;
    cancel(&transaction, scan_id)?;
    artists::refresh(&transaction, refreshed_at_ms)?;
    transaction.commit()?;
    Ok(())
}

pub fn finish_offline(
    transaction: &LibraryTransaction<'_>,
    scan_id: i64,
    storage_root_id: StorageRootId,
    finished_at_ms: i64,
    error_message: &str,
) -> Result<(), LibraryError> {
    storage_roots::record_scan_outcome(transaction, storage_root_id, finished_at_ms, Some(false))?;
    transaction.inner.execute(
        "UPDATE scan_history
         SET finished_at_ms = ?2, error_count = 1, outcome = 'offline',
             error_message = ?3
         WHERE id = ?1",
        params![scan_id, finished_at_ms, error_message],
    )?;
    Ok(())
}

pub fn finish_offline_and_refresh(
    store: &mut LibraryStore,
    scan_id: i64,
    storage_root_id: StorageRootId,
    finished_at_ms: i64,
    error_message: &str,
) -> Result<(), LibraryError> {
    let transaction = store.transaction()?;
    finish_offline(
        &transaction,
        scan_id,
        storage_root_id,
        finished_at_ms,
        error_message,
    )?;
    artists::refresh(&transaction, finished_at_ms)?;
    transaction.commit()?;
    Ok(())
}

pub fn finish_failed(
    transaction: &LibraryTransaction<'_>,
    scan_id: i64,
    storage_root_id: StorageRootId,
    finished_at_ms: i64,
    error_message: &str,
) -> Result<(), LibraryError> {
    storage_roots::record_scan_outcome(transaction, storage_root_id, finished_at_ms, None)?;
    transaction.inner.execute(
        "UPDATE scan_history
         SET finished_at_ms = ?2, error_count = 1, outcome = 'failed',
             error_message = ?3
         WHERE id = ?1",
        params![scan_id, finished_at_ms, error_message],
    )?;
    Ok(())
}

pub fn finish_failed_and_refresh(
    store: &mut LibraryStore,
    scan_id: i64,
    storage_root_id: StorageRootId,
    finished_at_ms: i64,
    error_message: &str,
) -> Result<(), LibraryError> {
    let transaction = store.transaction()?;
    finish_failed(
        &transaction,
        scan_id,
        storage_root_id,
        finished_at_ms,
        error_message,
    )?;
    artists::refresh(&transaction, finished_at_ms)?;
    transaction.commit()?;
    Ok(())
}

pub fn finish_completed_scan(
    transaction: &LibraryTransaction<'_>,
    scan_id: i64,
    storage_root_id: StorageRootId,
    completed: &CompletedScan,
) -> Result<(), LibraryError> {
    storage_roots::record_scan_outcome(
        transaction,
        storage_root_id,
        completed.finished_at_ms,
        Some(true),
    )?;
    transaction.inner.execute(
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

pub fn finish_completed_scan_and_refresh(
    store: &mut LibraryStore,
    scan_id: i64,
    storage_root_id: StorageRootId,
    completed: &CompletedScan,
) -> Result<(), LibraryError> {
    let transaction = store.transaction()?;
    finish_completed_scan(&transaction, scan_id, storage_root_id, completed)?;
    artists::refresh(&transaction, completed.finished_at_ms)?;
    transaction.commit()?;
    Ok(())
}

pub fn recent(
    store: &LibraryStore,
    storage_root_id: StorageRootId,
    limit: usize,
) -> Result<Vec<ScanHistoryEntry>, LibraryError> {
    let conn = &store.connection;
    let limit =
        i64::try_from(limit).map_err(|_| LibraryError::IntegerOutOfRange("scan history limit"))?;
    let columns = select_list(COLUMNS);
    let sql = format!(
        "SELECT {columns}
         FROM scan_history
         WHERE storage_root_id = ?1
         ORDER BY started_at_ms DESC, id DESC
         LIMIT ?2"
    );
    let mut statement = conn.prepare(&sql)?;
    let scans = statement
        .query_map(params![storage_root_id, limit], scan_history_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(scans)
}

/// Closes scans another session left unfinished — the app crashed or was
/// killed mid-scan — so they surface as failed instead of forever-running.
pub fn recover_interrupted(
    store: &mut LibraryStore,
    recovered_at_ms: i64,
) -> Result<(), LibraryError> {
    let conn = &store.connection;
    conn.execute(
        "UPDATE scan_history
         SET finished_at_ms = ?1,
             error_count = CASE WHEN error_count = 0 THEN 1 ELSE error_count END,
             outcome = 'failed',
             error_message = 'Scan interrupted before completion'
         WHERE finished_at_ms IS NULL
           AND outcome IS NULL
           AND scan_session_id <> ?2",
        params![recovered_at_ms, store.scan_session_id],
    )?;
    Ok(())
}

pub fn delete_for_root(
    transaction: &LibraryTransaction<'_>,
    storage_root_id: StorageRootId,
) -> Result<(), LibraryError> {
    transaction.inner.execute(
        "DELETE FROM scan_history WHERE storage_root_id = ?1",
        [storage_root_id],
    )?;
    Ok(())
}

fn scan_history_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanHistoryEntry> {
    let row = ScanHistoryRow {
        id: row.get(0)?,
        storage_root_id: row.get(1)?,
        _scan_session_id: row.get(2)?,
        started_at_ms: row.get(3)?,
        finished_at_ms: row.get(4)?,
        added_count: row.get(5)?,
        updated_count: row.get(6)?,
        removed_count: row.get(7)?,
        unsupported_count: row.get(8)?,
        error_count: row.get(9)?,
        removals_suppressed: row.get(10)?,
        outcome: row.get(11)?,
        error_message: row.get(12)?,
    };
    let outcome = row
        .outcome
        .map(|outcome| {
            ScanOutcome::from_db_str(&outcome).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    11,
                    rusqlite::types::Type::Text,
                    format!("unknown scan outcome {outcome}").into(),
                )
            })
        })
        .transpose()?;
    Ok(ScanHistoryEntry {
        id: row.id,
        storage_root_id: row.storage_root_id,
        started_at_ms: row.started_at_ms,
        finished_at_ms: row.finished_at_ms,
        added_count: row.added_count as u64,
        updated_count: row.updated_count as u64,
        removed_count: row.removed_count as u64,
        unsupported_count: row.unsupported_count as u64,
        error_count: row.error_count as u64,
        removals_suppressed: row.removals_suppressed,
        outcome,
        error_message: row.error_message,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::backend::{
        LibraryStore, ScanOutcome,
        repo::testing::{insert_track, test_file, test_metadata},
    };

    #[test]
    fn marks_an_offline_root_without_removing_its_tracks() {
        let temp = tempdir().unwrap();
        let mut store = LibraryStore::open_in_memory().unwrap();
        let root =
            crate::backend::repo::storage_roots::add(&mut store, temp.path(), "Music").unwrap();
        insert_track(
            &mut store,
            &root,
            &test_file(&root, "track.wav", 1, 10),
            &test_metadata("Track", "Artist", Some("Album"), None),
        );
        let scan_id = crate::backend::repo::scan_history::begin(&mut store, root.id, 100).unwrap();

        crate::backend::repo::scan_history::finish_offline_and_refresh(
            &mut store,
            scan_id,
            root.id,
            200,
            "not mounted",
        )
        .unwrap();

        assert_eq!(
            crate::backend::repo::tracks::for_root(&store, root.id)
                .unwrap()
                .len(),
            1
        );
        let stored_root = crate::backend::repo::storage_roots::get(&store, root.id)
            .unwrap()
            .unwrap();
        assert!(!stored_root.is_reachable);
        assert_eq!(stored_root.last_scan_at_ms, Some(200));
        let history = crate::backend::repo::scan_history::recent(&store, root.id, 1).unwrap();
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
        let root = super::storage_roots::add(&mut first, &music, "Music").unwrap();
        let root_id = root.id;
        let scan_id = super::begin(&mut first, root.id, 100).unwrap();
        drop(first);

        let same_session = LibraryStore::from_connection(
            Connection::open(&database_path).unwrap(),
            "first-session".to_string(),
            |_| {},
        )
        .unwrap();
        assert_eq!(
            super::recent(&same_session, root_id, 1).unwrap()[0].outcome,
            None
        );
        drop(same_session);

        let reopened = LibraryStore::from_connection(
            Connection::open(&database_path).unwrap(),
            "second-session".to_string(),
            |_| {},
        )
        .unwrap();
        assert_eq!(super::storage_roots::list(&reopened).unwrap(), vec![root]);
        let history = super::recent(&reopened, root_id, 1).unwrap();
        assert_eq!(history[0].id, scan_id);
        assert_eq!(history[0].outcome, Some(ScanOutcome::Failed));
        assert!(history[0].finished_at_ms.is_some());
        assert_eq!(
            history[0].error_message.as_deref(),
            Some("Scan interrupted before completion")
        );
    }
}
