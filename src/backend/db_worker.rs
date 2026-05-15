use crate::backend::backup_worker::BackupCommand;
use crate::backend::schema;
use crate::policies::{LoggingPolicy, RetryPolicy};
use crate::tasks::{
    CompanyDetails, CompanyRow, DailyLogEntry, DbTask, FetchCompaniesReq, FetchCompaniesResult,
    InsertCompanyReq, OperationResult, UiTask, UpdateCompanyReq,
};
use chrono::Local;
use crossbeam::channel::{Receiver, Sender};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::PathBuf;
use std::sync::Arc;

pub struct DbWorker<L: LoggingPolicy, R: RetryPolicy> {
    db_path: PathBuf,
    logger: Arc<L>,
    retry_policy: Arc<R>,
    backup_tx: Sender<BackupCommand>,
}

impl<L: LoggingPolicy, R: RetryPolicy> DbWorker<L, R> {
    pub fn new(
        db_path: PathBuf,
        logger: Arc<L>,
        retry_policy: Arc<R>,
        backup_tx: Sender<BackupCommand>,
    ) -> Self {
        Self {
            db_path,
            logger,
            retry_policy,
            backup_tx,
        }
    }

    pub fn run(&self, rx: Receiver<DbTask>, ui_tx: Sender<UiTask>) {
        if let Some(parent) = self.db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn_result = self
            .retry_policy
            .attempt(|| Connection::open(&self.db_path).map_err(|e| e.to_string()));
        let conn = match conn_result {
            Ok(conn) => conn,
            Err(err) => {
                self.logger
                    .log("ERROR", &format!("Failed to open database: {err}"));
                return;
            }
        };

        if let Err(err) = schema::bootstrap_schema(&conn) {
            self.logger
                .log("ERROR", &format!("Schema bootstrap failed: {err}"));
            return;
        }

        if let Err(err) = schema::apply_pragmas(&conn) {
            self.logger.log("ERROR", &format!("PRAGMA setup failed: {err}"));
            return;
        }

        self.logger.log("INFO", "DB worker started");

        while let Ok(task) = rx.recv() {
            match task {
                DbTask::FetchCompanies(req) => {
                    let result = self.handle_fetch(&conn, &req);
                    let _ = ui_tx.send(UiTask::FetchCompaniesResult(result));
                }
                DbTask::FetchCompanyDetails(company_id) => {
                    let result = self.handle_fetch_details(&conn, company_id);
                    let _ = ui_tx.send(UiTask::FetchCompanyDetailsResult(company_id, result));
                }
                DbTask::InsertCompany(req) => {
                    let result = self.handle_insert(&conn, &req);
                    let _ = ui_tx.send(UiTask::InsertCompanyResult(result));
                }
                DbTask::UpdateCompany(req) => {
                    let result = self.handle_update(&conn, &req);
                    let _ = ui_tx.send(UiTask::UpdateCompanyResult(result));
                }
                DbTask::DeleteCompany(company_id) => {
                    let result = self.handle_delete(&conn, company_id);
                    let _ = ui_tx.send(UiTask::DeleteCompanyResult(result));
                }
                DbTask::FetchTodayLog => {
                    let result = self.handle_fetch_today_log(&conn);
                    let _ = ui_tx.send(UiTask::FetchTodayLogResult(result));
                }
                DbTask::SaveTodayLog(entry) => {
                    let result = self.handle_save_today_log(&conn, &entry);
                    let _ = ui_tx.send(UiTask::SaveTodayLogResult(result));
                }
                DbTask::RequestBackup => {
                    let result = match self.backup_tx.send(BackupCommand::RunNow) {
                        Ok(_) => Ok(OperationResult {
                            success: true,
                            message: "Backup requested.".to_string(),
                        }),
                        Err(err) => Err(format!("Failed to request backup: {err}")),
                    };
                    let _ = ui_tx.send(UiTask::BackupStatusResult(result));
                }
                DbTask::Shutdown => {
                    self.logger.log("INFO", "DB worker shutting down");
                    break;
                }
            }
        }
    }

    fn handle_fetch(
        &self,
        conn: &Connection,
        req: &FetchCompaniesReq,
    ) -> Result<FetchCompaniesResult, String> {
        let search = req.search.trim();
        let fts_query = build_fts_query(search);

        let mut stmt = conn
            .prepare(
                r#"
                SELECT c.id, c.name
                FROM companies c
                WHERE
                    (?1 = '')
                    OR (LOWER(c.name) LIKE '%' || LOWER(?1) || '%')
                    OR (
                        ?2 != ''
                        AND c.rowid IN (
                            SELECT rowid FROM companies_fts
                            WHERE companies_fts MATCH ?2
                        )
                    )
                ORDER BY c.name COLLATE NOCASE
                LIMIT ?3 OFFSET ?4
                "#,
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(
                params![search, &fts_query, req.limit as i64, req.offset as i64],
                |row| {
                    Ok(CompanyRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let total: u32 = conn
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM companies c
                WHERE
                    (?1 = '')
                    OR (LOWER(c.name) LIKE '%' || LOWER(?1) || '%')
                OR (
                        ?2 != ''
                        AND c.rowid IN (
                            SELECT rowid FROM companies_fts
                            WHERE companies_fts MATCH ?2
                        )
                    )
                "#,
                params![search, &fts_query],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;

        Ok(FetchCompaniesResult { rows, total })
    }

    fn handle_fetch_details(&self, conn: &Connection, company_id: i64) -> Result<CompanyDetails, String> {
        let (name, county): (String, Option<String>) = conn
            .query_row(
                r#"
                SELECT c.name, co.name
                FROM companies c
                LEFT JOIN counties co ON co.id = c.county_id
                WHERE c.id = ?1
                "#,
                [company_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;

        let contact: Option<(String, Option<String>)> = conn
            .query_row(
                r#"
                SELECT ct.name, ct.last_name
                FROM company_contacts cc
                JOIN contacts ct ON ct.id = cc.contact_id
                WHERE cc.company_id = ?1
                LIMIT 1
                "#,
                [company_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        let (contact_first, contact_last) = match contact {
            Some((first, last)) => (first, last.unwrap_or_default()),
            None => (String::new(), String::new()),
        };

        Ok(CompanyDetails {
            id: company_id,
            name,
            county: county.unwrap_or_default(),
            contact_first,
            contact_last,
        })
    }

    fn handle_insert(
        &self,
        conn: &Connection,
        req: &InsertCompanyReq,
    ) -> Result<OperationResult, String> {
        let name = req.name.trim();
        if name.is_empty() {
            return Ok(OperationResult {
                success: false,
                message: "Company name is required.".to_string(),
            });
        }

        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| e.to_string())?;

        let county_id = if req.county.trim().is_empty() {
            None
        } else {
            match self.ensure_county(conn, req.county.trim()) {
                Ok(value) => Some(value),
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    return Err(err);
                }
            }
        };

        let insert_result = conn.execute(
            "INSERT INTO companies(name, county_id) VALUES(?1, ?2)",
            params![name, county_id],
        );

        let company_id = match insert_result {
            Ok(_) => conn.last_insert_rowid(),
            Err(e) if e.to_string().contains("UNIQUE") => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Ok(OperationResult {
                    success: false,
                    message: format!("Company '{}' already exists.", name),
                });
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Ok(OperationResult {
                    success: false,
                    message: format!("Error: {e}"),
                });
            }
        };

        if let Err(err) =
            self.upsert_company_contact(conn, company_id, &req.contact_first, &req.contact_last)
        {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err);
        }

        conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;

        Ok(OperationResult {
            success: true,
            message: format!("Company '{}' created.", name),
        })
    }

    fn handle_update(
        &self,
        conn: &Connection,
        req: &UpdateCompanyReq,
    ) -> Result<OperationResult, String> {
        let new_name = req.new_name.trim();
        if new_name.is_empty() {
            return Ok(OperationResult {
                success: false,
                message: "Company name is required.".to_string(),
            });
        }

        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| e.to_string())?;

        let county_id = if req.county.trim().is_empty() {
            None
        } else {
            match self.ensure_county(conn, req.county.trim()) {
                Ok(value) => Some(value),
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    return Err(err);
                }
            }
        };

        let update_result = conn.execute(
            "UPDATE companies SET name = ?1, county_id = ?2 WHERE id = ?3",
            params![new_name, county_id, req.id],
        );

        let affected = match update_result {
            Ok(rows) => rows,
            Err(e) if e.to_string().contains("UNIQUE") => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Ok(OperationResult {
                    success: false,
                    message: format!("Company '{}' already exists.", new_name),
                });
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Ok(OperationResult {
                    success: false,
                    message: format!("Error: {e}"),
                });
            }
        };

        if affected == 0 {
            let _ = conn.execute_batch("ROLLBACK;");
            return Ok(OperationResult {
                success: false,
                message: "Company not found.".to_string(),
            });
        }

        if let Err(err) = self.upsert_company_contact(conn, req.id, &req.contact_first, &req.contact_last)
        {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err);
        }

        conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;

        Ok(OperationResult {
            success: true,
            message: "Company updated.".to_string(),
        })
    }

    fn handle_delete(&self, conn: &Connection, company_id: i64) -> Result<OperationResult, String> {
        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| e.to_string())?;

        if let Err(err) = conn.execute(
            "DELETE FROM company_contacts WHERE company_id = ?1",
            [company_id],
        ) {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err.to_string());
        }

        if let Err(err) = conn.execute("DELETE FROM activities WHERE company_id = ?1", [company_id]) {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err.to_string());
        }

        let deleted = match conn.execute("DELETE FROM companies WHERE id = ?1", [company_id]) {
            Ok(rows) => rows,
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(err.to_string());
            }
        };

        if deleted == 0 {
            let _ = conn.execute_batch("ROLLBACK;");
            return Ok(OperationResult {
                success: false,
                message: "Company not found.".to_string(),
            });
        }

        conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;

        Ok(OperationResult {
            success: true,
            message: "Company deleted.".to_string(),
        })
    }

    fn handle_fetch_today_log(
        &self,
        conn: &Connection,
    ) -> Result<Option<DailyLogEntry>, String> {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let log = conn
            .query_row(
                "SELECT log_date, entry, created_at FROM daily_logs WHERE log_date = ?1 LIMIT 1",
                [today],
                |row| {
                    Ok(DailyLogEntry {
                        log_date: row.get(0)?,
                        entry: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(log)
    }

    fn handle_save_today_log(
        &self,
        conn: &Connection,
        entry: &str,
    ) -> Result<OperationResult, String> {
        let clean_entry = entry.trim();
        if clean_entry.is_empty() {
            return Ok(OperationResult {
                success: false,
                message: "Daily log entry cannot be empty.".to_string(),
            });
        }

        let today = Local::now().format("%Y-%m-%d").to_string();
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        conn.execute(
            r#"
            INSERT INTO daily_logs(log_date, entry, created_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(log_date) DO UPDATE SET
                entry = excluded.entry,
                created_at = excluded.created_at
            "#,
            params![today, clean_entry, now],
        )
        .map_err(|e| e.to_string())?;

        Ok(OperationResult {
            success: true,
            message: "Daily log saved.".to_string(),
        })
    }

    fn ensure_county(&self, conn: &Connection, county_name: &str) -> Result<i64, String> {
        conn.execute(
            "INSERT INTO counties(name) VALUES(?1) ON CONFLICT(name) DO NOTHING",
            [county_name],
        )
        .map_err(|e| e.to_string())?;

        conn.query_row(
            "SELECT id FROM counties WHERE name = ?1",
            [county_name],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    }

    fn upsert_company_contact(
        &self,
        conn: &Connection,
        company_id: i64,
        first_name: &str,
        last_name: &str,
    ) -> Result<(), String> {
        conn.execute(
            "DELETE FROM company_contacts WHERE company_id = ?1",
            [company_id],
        )
        .map_err(|e| e.to_string())?;

        let first = first_name.trim();
        let last = last_name.trim();
        if first.is_empty() && last.is_empty() {
            return Ok(());
        }

        conn.execute(
            "INSERT INTO contacts(name, last_name) VALUES(?1, ?2)",
            params![first, if last.is_empty() { None } else { Some(last) }],
        )
        .map_err(|e| e.to_string())?;

        let contact_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO company_contacts(company_id, contact_id) VALUES(?1, ?2)",
            params![company_id, contact_id],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    }
}

fn build_fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .filter_map(|token| {
            let cleaned: String = token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            if cleaned.is_empty() {
                None
            } else {
                Some(format!("{cleaned}*"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
