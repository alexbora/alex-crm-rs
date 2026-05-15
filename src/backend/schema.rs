use rusqlite::{Connection, Result};

pub fn bootstrap_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS companies (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL,
            county_id INTEGER REFERENCES counties(id)
        );

        CREATE TABLE IF NOT EXISTS counties (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL
        );

        CREATE TABLE IF NOT EXISTS contacts (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            last_name TEXT
        );

        CREATE TABLE IF NOT EXISTS company_contacts (
            company_id INTEGER NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
            contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
            PRIMARY KEY(company_id, contact_id)
        );

        CREATE TABLE IF NOT EXISTS activities (
            id INTEGER PRIMARY KEY,
            company_id INTEGER NOT NULL REFERENCES companies(id) ON DELETE CASCADE,
            type TEXT,
            description TEXT,
            created_at TEXT
        );

        CREATE TABLE IF NOT EXISTS daily_logs (
            id INTEGER PRIMARY KEY,
            log_date TEXT NOT NULL UNIQUE,
            entry TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS companies_fts USING fts5(
            name,
            content='companies',
            content_rowid='id',
            tokenize = 'porter'
        );

        CREATE TRIGGER IF NOT EXISTS companies_ai AFTER INSERT ON companies BEGIN
            INSERT INTO companies_fts(rowid, name) VALUES (new.id, new.name);
        END;

        CREATE TRIGGER IF NOT EXISTS companies_ad AFTER DELETE ON companies BEGIN
            INSERT INTO companies_fts(companies_fts, rowid, name)
            VALUES ('delete', old.id, old.name);
        END;

        CREATE TRIGGER IF NOT EXISTS companies_au AFTER UPDATE ON companies BEGIN
            INSERT INTO companies_fts(companies_fts, rowid, name)
            VALUES ('delete', old.id, old.name);
            INSERT INTO companies_fts(rowid, name) VALUES (new.id, new.name);
        END;
        "#,
    )?;

    Ok(())
}

pub fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = 5000;
        "#,
    )?;
    Ok(())
}
