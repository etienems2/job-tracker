use jiff::civil::Date;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use thiserror::Error;

use crate::domain::{
    Application, ApplicationId, EventId, MAX_NAME_CHARS, NameIssue, Op, Snapshot, Stage,
    StageEvent, normalize_name,
};

const SQLITE_BUSY_TIMEOUT: i64 = 3000;

const SUPPORTED_VERSION: i64 = 2;

const MIGRATION_V1: &str = r#"
CREATE TABLE applications (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    company    TEXT NOT NULL CHECK (length(company) BETWEEN 1 AND 200),
    role       TEXT NOT NULL CHECK (length(role) BETWEEN 1 AND 200)
) STRICT;

CREATE TABLE stage_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id  INTEGER NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    stage           TEXT NOT NULL CHECK (stage IN (
                        'applied',
                        'first_meeting',
                        'second_meeting',
                        'third_meeting',
                        'technical_meeting',
                        'refused',
                        'offer_made',
                        'offer_accepted'
                    )),
    occurred_on     TEXT NOT NULL CHECK (occurred_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]')
) STRICT;

CREATE INDEX stage_events_by_app
    ON stage_events (application_id, occurred_on, id);
"#;

const MIGRATION_V2: &str = r#"
-- `1970-01-01` exists only so NOT NULL is legal on an added column.
-- The UPDATE below replaces it before the migration commits.
ALTER TABLE applications ADD COLUMN applied_on TEXT NOT NULL DEFAULT '1970-01-01'
    CHECK (applied_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]');

UPDATE applications
SET applied_on = (
    SELECT occurred_on FROM stage_events
    WHERE application_id = applications.id
    ORDER BY occurred_on, id
    LIMIT 1
);
"#;

#[derive(Debug, Error)]
pub(crate) enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("database version {found} is newer than this program (supports {supported})")]
    Newer { found: i64, supported: i64 },
}

pub(crate) trait Store {
    fn load(&self) -> Result<Snapshot, StoreError>;
    fn apply(&mut self, op: Op) -> Result<Snapshot, StoreError>;
    fn export_json(&self) -> Result<String, StoreError>;
}

pub(crate) struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub(crate) fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_configured(Connection::open(path)?, true)
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_configured(Connection::open_in_memory()?, false)
    }

    fn from_configured(conn: Connection, wal: bool) -> Result<Self, StoreError> {
        configure(&conn, wal)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub(crate) fn insert_all(&self, ops: &[Op]) -> Result<Snapshot, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        for op in ops {
            let Op::Add {
                company,
                role,
                stage,
                on,
            } = op
            else {
                return Err(StoreError::Invalid(
                    "import only inserts new applications".into(),
                ));
            };
            add(&tx, company, role, *stage, *on)?;
        }
        tx.commit()?;
        self.load()
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let version = user_version(&self.conn)?;
        if version == SUPPORTED_VERSION {
            return Ok(());
        }
        if version > SUPPORTED_VERSION {
            return Err(StoreError::Newer {
                found: version,
                supported: SUPPORTED_VERSION,
            });
        }
        if version > 1 {
            return Err(StoreError::Invalid(format!(
                "unsupported database version {version}"
            )));
        }
        let tx = self.conn.unchecked_transaction()?;
        if version == 0 {
            tx.execute_batch(MIGRATION_V1)?;
        }
        tx.execute_batch(MIGRATION_V2)?;
        tx.pragma_update(None, "user_version", SUPPORTED_VERSION)?;
        tx.commit()?;
        Ok(())
    }
}

impl Store for SqliteStore {
    fn load(&self) -> Result<Snapshot, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.company, a.role, a.applied_on, e.id, e.stage, e.occurred_on
             FROM applications a
             INNER JOIN stage_events e ON e.application_id = a.id
             ORDER BY a.id ASC, e.occurred_on ASC, e.id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(LoadedRow {
                app_id: row.get(0)?,
                company: row.get(1)?,
                role: row.get(2)?,
                applied_on: row.get(3)?,
                event_id: row.get(4)?,
                stage: row.get(5)?,
                occurred_on: row.get(6)?,
            })
        })?;

        let mut applications: Vec<Application> = Vec::new();
        for row in rows {
            let row = row?;
            let stage = Stage::from_token(&row.stage)
                .ok_or_else(|| StoreError::Invalid(format!("unknown stage '{}'", row.stage)))?;
            let occurred_on = parse_stored_date(&row.occurred_on)?;
            let event = StageEvent {
                id: EventId::new(row.event_id),
                stage,
                occurred_on,
            };
            if let Some(app) = applications
                .last_mut()
                .filter(|app| app.id == ApplicationId::new(row.app_id))
            {
                app.events.push(event);
            } else {
                let mut app = Application::new(
                    ApplicationId::new(row.app_id),
                    row.company,
                    row.role,
                    vec![event],
                );
                app.applied_on = parse_stored_date(&row.applied_on)?;
                applications.push(app);
            }
        }
        for app in &mut applications {
            app.sort_events();
        }
        Ok(Snapshot { applications })
    }

    fn apply(&mut self, op: Op) -> Result<Snapshot, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        apply_op(&tx, &op)?;
        tx.commit()?;
        self.load()
    }

    fn export_json(&self) -> Result<String, StoreError> {
        let snapshot = self.load()?;
        let exported: Vec<ExportApplication> = snapshot
            .applications
            .into_iter()
            .map(|app| ExportApplication {
                id: app.id.0,
                company: app.company,
                role: app.role,
                applied_on: app.applied_on.to_string(),
                events: app
                    .events
                    .into_iter()
                    .map(|event| ExportEvent {
                        id: event.id.0,
                        stage: event.stage.token().to_string(),
                        date: event.occurred_on.to_string(),
                    })
                    .collect(),
            })
            .collect();
        Ok(serde_json::to_string_pretty(&exported)?)
    }
}

struct LoadedRow {
    app_id: i64,
    company: String,
    role: String,
    applied_on: String,
    event_id: i64,
    stage: String,
    occurred_on: String,
}

#[derive(Serialize)]
struct ExportApplication {
    id: i64,
    company: String,
    role: String,
    applied_on: String,
    events: Vec<ExportEvent>,
}

#[derive(Serialize)]
struct ExportEvent {
    id: i64,
    stage: String,
    date: String,
}

fn configure(conn: &Connection, wal: bool) -> Result<(), StoreError> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    if wal {
        conn.pragma_update(None, "journal_mode", "WAL")?;
    }
    conn.pragma_update(None, "busy_timeout", SQLITE_BUSY_TIMEOUT)?;
    Ok(())
}

fn user_version(conn: &Connection) -> Result<i64, rusqlite::Error> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
}

fn apply_op(conn: &Connection, op: &Op) -> Result<(), StoreError> {
    match op {
        Op::Add {
            company,
            role,
            stage,
            on,
        } => add(conn, company, role, *stage, *on),
        Op::SetStage {
            application_id,
            stage,
            on,
        } => set_stage(conn, *application_id, *stage, *on),
        Op::Rename {
            application_id,
            company,
            role,
        } => rename(conn, *application_id, company, role),
        Op::Delete { application_id } => delete_application(conn, *application_id),
        Op::AmendEvent {
            event_id,
            stage,
            on,
        } => amend_event(conn, *event_id, *stage, *on),
        Op::DeleteEvent { event_id } => delete_event(conn, *event_id),
    }
}

fn add(
    conn: &Connection,
    company: &str,
    role: &str,
    stage: Stage,
    on: Date,
) -> Result<(), StoreError> {
    let company = require_name("company", company)?;
    let role = require_name("role", role)?;
    let date = sql_date(on)?;
    conn.execute(
        "INSERT INTO applications (company, role) VALUES (?1, ?2)",
        params![company, role],
    )?;
    let application_id = ApplicationId::new(conn.last_insert_rowid());
    insert_event(conn, application_id, stage, &date)?;
    refresh_applied_on(conn, application_id)
}

fn set_stage(
    conn: &Connection,
    application_id: ApplicationId,
    stage: Stage,
    on: Date,
) -> Result<(), StoreError> {
    if !application_exists(conn, application_id)? {
        return Err(StoreError::Invalid(format!(
            "no application {application_id}"
        )));
    }
    let date = sql_date(on)?;
    insert_event(conn, application_id, stage, &date)?;
    refresh_applied_on(conn, application_id)
}

fn rename(
    conn: &Connection,
    application_id: ApplicationId,
    company: &str,
    role: &str,
) -> Result<(), StoreError> {
    let company = require_name("company", company)?;
    let role = require_name("role", role)?;
    let changed = conn.execute(
        "UPDATE applications SET company = ?1, role = ?2 WHERE id = ?3",
        params![company, role, application_id.0],
    )?;
    if changed == 0 {
        return Err(StoreError::Invalid(format!(
            "no application {application_id}"
        )));
    }
    Ok(())
}

fn delete_application(conn: &Connection, application_id: ApplicationId) -> Result<(), StoreError> {
    let changed = conn.execute("DELETE FROM applications WHERE id = ?1", [application_id.0])?;
    if changed == 0 {
        return Err(StoreError::Invalid(format!(
            "no application {application_id}"
        )));
    }
    Ok(())
}

fn amend_event(
    conn: &Connection,
    event_id: EventId,
    stage: Stage,
    on: Date,
) -> Result<(), StoreError> {
    let date = sql_date(on)?;
    let application_id = event_application_id(conn, event_id)?;
    let changed = conn.execute(
        "UPDATE stage_events SET stage = ?1, occurred_on = ?2 WHERE id = ?3",
        params![stage.token(), date, event_id.0],
    )?;
    if changed == 0 {
        return Err(StoreError::Invalid(format!("no event {event_id}")));
    }
    refresh_applied_on(conn, application_id)
}

fn delete_event(conn: &Connection, event_id: EventId) -> Result<(), StoreError> {
    let application_id = event_application_id(conn, event_id)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM stage_events WHERE application_id = ?1",
        [application_id.0],
        |row| row.get(0),
    )?;
    if count <= 1 {
        return Err(StoreError::Invalid(
            "cannot delete the only stage event; delete the application instead".into(),
        ));
    }
    conn.execute("DELETE FROM stage_events WHERE id = ?1", [event_id.0])?;
    refresh_applied_on(conn, application_id)
}

fn event_application_id(conn: &Connection, event_id: EventId) -> Result<ApplicationId, StoreError> {
    let id = conn
        .query_row(
            "SELECT application_id FROM stage_events WHERE id = ?1",
            [event_id.0],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| StoreError::Invalid(format!("no event {event_id}")))?;
    Ok(ApplicationId::new(id))
}

fn refresh_applied_on(conn: &Connection, application_id: ApplicationId) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE applications
         SET applied_on = (
             SELECT occurred_on FROM stage_events
             WHERE application_id = ?1
             ORDER BY occurred_on, id
             LIMIT 1
         )
         WHERE id = ?1",
        [application_id.0],
    )?;
    Ok(())
}

fn parse_stored_date(text: &str) -> Result<Date, StoreError> {
    text.parse()
        .map_err(|_| StoreError::Invalid(format!("bad date '{text}'")))
}

fn insert_event(
    conn: &Connection,
    application_id: ApplicationId,
    stage: Stage,
    date: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO stage_events (application_id, stage, occurred_on) VALUES (?1, ?2, ?3)",
        params![application_id.0, stage.token(), date],
    )?;
    Ok(())
}

fn application_exists(conn: &Connection, id: ApplicationId) -> Result<bool, StoreError> {
    let found = conn
        .query_row("SELECT 1 FROM applications WHERE id = ?1", [id.0], |_| {
            Ok(())
        })
        .optional()?;
    Ok(found.is_some())
}

fn require_name(kind: &str, value: &str) -> Result<String, StoreError> {
    normalize_name(value).map_err(|issue| match issue {
        NameIssue::Empty => StoreError::Invalid(format!("{kind} is empty")),
        NameIssue::TooLong => {
            StoreError::Invalid(format!("{kind} is longer than {MAX_NAME_CHARS} characters"))
        }
    })
}

fn sql_date(date: Date) -> Result<String, StoreError> {
    let text = date.to_string();
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit());
    if shaped {
        Ok(text)
    } else {
        Err(StoreError::Invalid(format!("date {text} cannot be stored")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn day(day: i8) -> Date {
        Date::new(2026, 11, day).unwrap()
    }

    fn memory() -> SqliteStore {
        SqliteStore::open_in_memory().expect("open memory database")
    }

    fn add(company: &str, role: &str, stage: Stage, on: Date) -> Op {
        Op::Add {
            company: company.into(),
            role: role.into(),
            stage,
            on,
        }
    }

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("job-tracker-{}-{n}", std::process::id()));
            Self(path)
        }

        fn file(&self) -> std::path::PathBuf {
            self.0.join("tracker.db")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn add_inserts_the_application_and_its_first_event_together() {
        let mut store = memory();
        let snapshot = store
            .apply(add(
                "  Acme  ",
                " Backend engineer ",
                Stage::Applied,
                day(1),
            ))
            .unwrap();
        assert_eq!(snapshot.applications.len(), 1);
        let app = &snapshot.applications[0];
        assert_eq!(app.company, "Acme");
        assert_eq!(app.role, "Backend engineer");
        assert_eq!(app.events.len(), 1);
        assert_eq!(app.events[0].stage, Stage::Applied);
        assert_eq!(app.events[0].occurred_on, day(1));
        assert!(app.is_active());
    }

    #[test]
    fn the_same_company_and_role_may_be_reapplied() {
        let mut store = memory();
        store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let snapshot = store
            .apply(add("Acme", "Backend", Stage::Applied, day(2)))
            .unwrap();
        assert_eq!(snapshot.applications.len(), 2);
        assert_ne!(snapshot.applications[0].id, snapshot.applications[1].id);
    }

    #[test]
    fn set_stage_appends_and_current_is_latest_date_then_id() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let id = created.applications[0].id;
        let snapshot = store
            .apply(Op::SetStage {
                application_id: id,
                stage: Stage::FirstMeeting,
                on: day(1),
            })
            .unwrap();
        let app = &snapshot.applications[0];
        assert_eq!(app.events.len(), 2);
        assert!(app.events[0].id < app.events[1].id);
        assert_eq!(app.current().unwrap().stage, Stage::FirstMeeting);

        let first_id = app.events[0].id;
        let amended = store
            .apply(Op::AmendEvent {
                event_id: first_id,
                stage: Stage::Refused,
                on: day(20),
            })
            .unwrap();
        let app = &amended.applications[0];
        assert_eq!(app.current().unwrap().id, first_id);
        assert_eq!(app.current().unwrap().stage, Stage::Refused);
        assert_eq!(app.current().unwrap().occurred_on, day(20));
    }

    #[test]
    fn rename_does_not_append_a_stage_event() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let id = created.applications[0].id;
        let snapshot = store
            .apply(Op::Rename {
                application_id: id,
                company: " Acme Corp ".into(),
                role: "Backend engineer".into(),
            })
            .unwrap();
        let app = &snapshot.applications[0];
        assert_eq!(app.company, "Acme Corp");
        assert_eq!(app.role, "Backend engineer");
        assert_eq!(app.events.len(), 1);
    }

    #[test]
    fn delete_event_refuses_the_last_event_and_rolls_back() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let event_id = created.applications[0].events[0].id;
        let error = store.apply(Op::DeleteEvent { event_id }).unwrap_err();
        assert!(error.to_string().contains("only stage event"), "{error}");
        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.applications[0].events.len(), 1);
        assert_eq!(snapshot.applications[0].events[0].id, event_id);
    }

    #[test]
    fn delete_event_removes_one_past_row_and_updates_the_current_stage() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let id = created.applications[0].id;
        let first = created.applications[0].events[0].id;
        let with_second = store
            .apply(Op::SetStage {
                application_id: id,
                stage: Stage::TechnicalMeeting,
                on: day(17),
            })
            .unwrap();
        let second = with_second.applications[0].current().unwrap().id;
        let snapshot = store.apply(Op::DeleteEvent { event_id: first }).unwrap();
        assert_eq!(snapshot.applications[0].events.len(), 1);
        assert_eq!(snapshot.applications[0].current().unwrap().id, second);
        assert_eq!(
            snapshot.applications[0].current().unwrap().stage,
            Stage::TechnicalMeeting
        );
    }

    #[test]
    fn delete_application_cascades_its_events() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let id = created.applications[0].id;
        store
            .apply(Op::SetStage {
                application_id: id,
                stage: Stage::Refused,
                on: day(3),
            })
            .unwrap();
        let snapshot = store.apply(Op::Delete { application_id: id }).unwrap();
        assert!(snapshot.applications.is_empty());
        let events: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM stage_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(events, 0);
    }

    #[test]
    fn missing_rows_and_invalid_names_do_not_write() {
        let mut store = memory();
        let missing = store
            .apply(Op::SetStage {
                application_id: ApplicationId::new(9),
                stage: Stage::Applied,
                on: day(1),
            })
            .unwrap_err();
        assert!(missing.to_string().contains("no application 9"));

        let empty = store
            .apply(add("   ", "Role", Stage::Applied, day(1)))
            .unwrap_err();
        assert!(empty.to_string().contains("company is empty"));

        let long = "a".repeat(MAX_NAME_CHARS + 1);
        let too_long = store
            .apply(add("Acme", &long, Stage::Applied, day(1)))
            .unwrap_err();
        assert!(too_long.to_string().contains("role is longer than 200"));

        assert!(store.load().unwrap().applications.is_empty());
    }

    #[test]
    fn future_dates_are_stored() {
        let mut store = memory();
        let on = Date::new(2027, 1, 15).unwrap();
        let snapshot = store
            .apply(add("Acme", "Backend", Stage::SecondMeeting, on))
            .unwrap();
        assert_eq!(snapshot.applications[0].current().unwrap().occurred_on, on);
    }

    #[test]
    fn any_stage_may_follow_any_stage() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let id = created.applications[0].id;
        let refused = store
            .apply(Op::SetStage {
                application_id: id,
                stage: Stage::Refused,
                on: day(2),
            })
            .unwrap();
        assert_eq!(
            refused.applications[0].current().unwrap().stage,
            Stage::Refused
        );
        let offered = store
            .apply(Op::SetStage {
                application_id: id,
                stage: Stage::OfferMade,
                on: day(3),
            })
            .unwrap();
        assert_eq!(
            offered.applications[0].current().unwrap().stage,
            Stage::OfferMade
        );
        assert!(offered.applications[0].is_active());
    }

    #[test]
    fn export_json_uses_the_documented_shape() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend engineer", Stage::Applied, day(1)))
            .unwrap();
        let id = created.applications[0].id;
        store
            .apply(Op::SetStage {
                application_id: id,
                stage: Stage::FirstMeeting,
                on: day(14),
            })
            .unwrap();
        let json = store.export_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value,
            serde_json::json!([
                {
                    "id": 1,
                    "company": "Acme",
                    "role": "Backend engineer",
                    "applied_on": "2026-11-01",
                    "events": [
                        { "id": 1, "stage": "applied", "date": "2026-11-01" },
                        { "id": 2, "stage": "first_meeting", "date": "2026-11-14" }
                    ]
                }
            ])
        );
        assert!(json.contains('\n'));
    }

    #[test]
    fn schema_rejects_an_unknown_stage_token() {
        let store = memory();
        store
            .conn
            .execute(
                "INSERT INTO applications (company, role) VALUES ('Acme', 'Backend')",
                [],
            )
            .unwrap();
        let error = store.conn.execute(
            "INSERT INTO stage_events (application_id, stage, occurred_on)
             VALUES (1, 'nope', '2026-11-01')",
            [],
        );
        assert!(error.is_err());
    }

    #[test]
    fn memory_database_skips_wal_and_records_the_migration() {
        let store = memory();
        let journal: String = store
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "memory");
        let version = user_version(&store.conn).unwrap();
        assert_eq!(version, 2);
        let foreign_keys: i64 = store
            .conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
        store.migrate().unwrap();
    }

    #[test]
    fn a_newer_user_version_is_rejected() {
        let store = memory();
        store
            .conn
            .pragma_update(None, "user_version", 3_i64)
            .unwrap();
        let error = store.migrate().unwrap_err();
        assert!(error.to_string().contains("newer"), "{error}");
    }

    #[test]
    fn file_database_uses_wal_and_reopens() {
        let dir = TempDir::new();
        let path = dir.file();
        {
            let mut store = SqliteStore::open(&path).unwrap();
            let journal: String = store
                .conn
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            assert_eq!(journal, "wal");
            let timeout: i64 = store
                .conn
                .pragma_query_value(None, "busy_timeout", |row| row.get(0))
                .unwrap();
            assert_eq!(timeout, SQLITE_BUSY_TIMEOUT);
            store
                .apply(add("Acme", "Backend", Stage::Applied, day(1)))
                .unwrap();
        }
        let store = SqliteStore::open(&path).unwrap();
        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.applications.len(), 1);
        assert_eq!(snapshot.applications[0].company, "Acme");
        assert!(path.is_file());
    }

    #[test]
    fn insert_all_adds_each_row_as_its_own_application() {
        let store = memory();
        let snapshot = store
            .insert_all(&[
                add("Acme", "Backend", Stage::Applied, day(1)),
                add("Acme", "Backend", Stage::TechnicalMeeting, day(14)),
            ])
            .unwrap();
        assert_eq!(snapshot.applications.len(), 2);
        assert_ne!(snapshot.applications[0].id, snapshot.applications[1].id);
        assert_eq!(snapshot.applications[0].company, "Acme");
        assert_eq!(
            snapshot.applications[1].current().unwrap().stage,
            Stage::TechnicalMeeting
        );
    }

    #[test]
    fn insert_all_rolls_back_when_a_later_row_is_invalid() {
        let store = memory();
        let error = store
            .insert_all(&[
                add("Acme", "Backend", Stage::Applied, day(1)),
                add("   ", "Backend", Stage::Applied, day(2)),
            ])
            .unwrap_err();
        assert!(error.to_string().contains("company is empty"), "{error}");
        assert!(store.load().unwrap().applications.is_empty());
    }

    #[test]
    fn a_later_stage_keeps_applied_on_and_moves_since() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let id = created.applications[0].id;
        assert_eq!(created.applications[0].applied_on, day(1));
        let staged = store
            .apply(Op::SetStage {
                application_id: id,
                stage: Stage::FirstMeeting,
                on: day(14),
            })
            .unwrap();
        let app = &staged.applications[0];
        assert_eq!(app.applied_on, day(1));
        assert_eq!(app.current().unwrap().occurred_on, day(14));
    }

    #[test]
    fn editing_the_earliest_event_moves_applied_on() {
        let mut store = memory();
        let created = store
            .apply(add("Acme", "Backend", Stage::Applied, day(1)))
            .unwrap();
        let id = created.applications[0].id;
        let earliest = created.applications[0].events[0].id;
        let with_second = store
            .apply(Op::SetStage {
                application_id: id,
                stage: Stage::FirstMeeting,
                on: day(14),
            })
            .unwrap();
        let later = with_second.applications[0].current().unwrap().id;

        let amended = store
            .apply(Op::AmendEvent {
                event_id: earliest,
                stage: Stage::Applied,
                on: day(20),
            })
            .unwrap();
        assert_eq!(amended.applications[0].applied_on, day(14));

        let deleted = store.apply(Op::DeleteEvent { event_id: later }).unwrap();
        assert_eq!(deleted.applications[0].applied_on, day(20));
        assert_eq!(deleted.applications[0].events.len(), 1);
    }

    #[test]
    fn version_1_database_backfills_applied_on() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.pragma_update(None, "user_version", 1_i64).unwrap();
        conn.execute(
            "INSERT INTO applications (company, role) VALUES ('Acme', 'Backend')",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO stage_events (application_id, stage, occurred_on)
             VALUES (?1, 'first_meeting', '2026-11-14')",
            [id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO stage_events (application_id, stage, occurred_on)
             VALUES (?1, 'applied', '2026-11-01')",
            [id],
        )
        .unwrap();

        let store = SqliteStore::from_configured(conn, false).unwrap();
        assert_eq!(user_version(&store.conn).unwrap(), 2);
        let app = &store.load().unwrap().applications[0];
        assert_eq!(app.applied_on, day(1));
        assert_eq!(app.current().unwrap().occurred_on, day(14));
    }

    #[test]
    fn amend_and_delete_missing_events_report_the_id() {
        let mut store = memory();
        let amended = store
            .apply(Op::AmendEvent {
                event_id: EventId::new(4),
                stage: Stage::Applied,
                on: day(1),
            })
            .unwrap_err();
        assert!(amended.to_string().contains("no event 4"));
        let deleted = store
            .apply(Op::DeleteEvent {
                event_id: EventId::new(4),
            })
            .unwrap_err();
        assert!(deleted.to_string().contains("no event 4"));
    }
}
