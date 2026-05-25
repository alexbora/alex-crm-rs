use crate::policies::{LoggingPolicy, RetryPolicy};
use crate::state::AppState;
use crate::tasks::{DbTask, FetchCompaniesReq, UiTask};
use crate::ui::{companies_tab, detail_window, main_window, theme};
use fltk::prelude::{InputExt, WidgetExt};
use std::sync::Arc;

pub struct App<L: LoggingPolicy, R: RetryPolicy> {
    logger: Arc<L>,
    retry_policy: Arc<R>,
    state: AppState,
}

impl<L: LoggingPolicy, R: RetryPolicy> App<L, R> {
    pub fn new(logger: Arc<L>, retry_policy: Arc<R>, state: AppState) -> Self {
        Self {
            logger,
            retry_policy,
            state,
        }
    }

    pub fn run(&self) {
        let app = fltk::app::App::default().with_scheme(fltk::app::Scheme::Base);
        theme::install();
        let mut ui = main_window::build_main_window(self.state.db_tx.clone());
        ui.window.show();

        self.fetch_companies(ui.companies.search_input.value());
        let _ = self.state.db_tx.send(DbTask::FetchTodayLog);

        while app.wait() {
            self.drain_ui_queue(&mut ui);
        }
    }

    fn fetch_companies(&self, search: String) {
        let req = FetchCompaniesReq {
            search,
            offset: 0,
            limit: 2_000_000,
        };
        let tx = self.state.db_tx.clone();
        if let Err(err) = self.retry_policy.attempt(|| {
            tx.send(DbTask::FetchCompanies(req.clone()))
                .map_err(|e| e.to_string())
        }) {
            self.logger.log(
                "ERROR",
                &format!("Failed to queue fetch companies task: {err}"),
            );
        }
    }

    fn refresh_companies(&self, ui: &main_window::MainWindowUi) {
        let _ = ui;
        self.fetch_companies(String::new());
    }

    fn set_status(&self, _ui: &mut main_window::MainWindowUi, text: &str) {
        self.logger.log("INFO", text);
    }

    fn drain_ui_queue(&self, ui: &mut main_window::MainWindowUi) {
        while let Ok(msg) = self.state.ui_rx.try_recv() {
            match msg {
                UiTask::FetchCompaniesResult(result) => match result {
                    Ok(data) => {
companies_tab::handle_companies_result(
    &mut ui.companies.browser,
    &ui.companies.all_companies,
    &ui.companies.company_rows,
    &data.rows,
    &ui.companies.search_input.value(),
    &mut ui.companies.status_frame,
    ui.companies.total.clone(),
    ui.companies.offset.clone(),
);
                        self.set_status(
                            ui,
                            &format!(
                                "Loaded {} companies ({} total).",
                                data.rows.len(),
                                data.total
                            ),
                        );
                    }
                    Err(err) => {
                        self.set_status(ui, &format!("Failed to fetch companies: {err}"));
                    }
                },
                UiTask::FetchCompanyDetailsResult(company_id, result) => match result {
                    Ok(details) => {
                        detail_window::apply_company_details(
                            &ui.companies.detail_windows,
                            company_id,
                            &details,
                        );
                    }
                    Err(err) => self.set_status(ui, &format!("Failed to load details: {err}")),
                },
                UiTask::InsertCompanyResult(result) => match result {
                    Ok(op) => {
                        self.set_status(ui, &op.message);
                        self.refresh_companies(ui);
                    }
                    Err(err) => self.set_status(ui, &format!("Insert failed: {err}")),
                },
                UiTask::UpdateCompanyResult(result) => match result {
                    Ok(op) => {
                        self.set_status(ui, &op.message);
                        self.refresh_companies(ui);
                    }
                    Err(err) => self.set_status(ui, &format!("Update failed: {err}")),
                },
                UiTask::DeleteCompanyResult(result) => match result {
                    Ok(op) => {
                        self.set_status(ui, &op.message);
                        self.refresh_companies(ui);
                    }
                    Err(err) => self.set_status(ui, &format!("Delete failed: {err}")),
                },
                UiTask::FetchTodayLogResult(result) => match result {
                    Ok(Some(entry)) => {
                        ui.logs_editor.set_value(&entry.entry);
                        ui.logs_status.set_label("Loaded today's log entry.");
                    }
                    Ok(None) => {
                        ui.logs_editor.set_value("");
                        ui.logs_status.set_label("No daily log yet for today.");
                    }
                    Err(err) => ui
                        .logs_status
                        .set_label(&format!("Failed to load log: {err}")),
                },
                UiTask::SaveTodayLogResult(result) => match result {
                    Ok(op) => ui.logs_status.set_label(&op.message),
                    Err(err) => ui.logs_status.set_label(&format!("Save failed: {err}")),
                },
                UiTask::BackupStatusResult(result) => match result {
                    Ok(op) => self.set_status(ui, &op.message),
                    Err(err) => self.set_status(ui, &format!("Backup error: {err}")),
                },
            }
        }
    }
}
