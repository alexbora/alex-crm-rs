use crate::tasks::{CompanyRow, DbTask, FetchCompaniesReq};
use crate::ui::{detail_window, new_company_form, theme};
use crossbeam::channel::Sender;
use fltk::{app, browser, button, enums::CallbackTrigger, frame, group, input, prelude::*, window};
use std::cell::RefCell;
use std::rc::Rc;

pub struct CompaniesTabUi {
    pub status_frame: frame::Frame,
    _group: group::Group,
    pub search_input: input::Input,
    pub browser: browser::HoldBrowser,

    pub all_companies: Rc<RefCell<Vec<CompanyRow>>>,
    pub company_rows: Rc<RefCell<Vec<CompanyRow>>>,
    pub detail_windows: detail_window::DetailWindowStore,
    _form_windows: Rc<RefCell<Vec<window::Window>>>,
    pub total: Rc<RefCell<u32>>,
    pub offset: Rc<RefCell<u32>>,
    #[allow(dead_code)]
    pub search_term: Rc<RefCell<String>>,
}

pub fn build_companies_tab(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    db_tx: Sender<DbTask>,
) -> CompaniesTabUi {
    let mut group = group::Group::new(x, y, w, h, "Companie");
    theme::style_tab_panel(&mut group);

    let mut search_label = frame::Frame::new(x + 20, y + 24, 70, 24, "Search");
    theme::style_toolbar_label(&mut search_label);
    let mut search_input = input::Input::new(x + 96, y + 20, w - 408, 36, "");
    theme::style_text_input(&mut search_input);
    search_input.set_trigger(CallbackTrigger::Changed);

    let mut new_button = button::Button::new(x + w - 292, y + 20, 104, 36, "+ New");
    theme::style_primary_button(&mut new_button);
    let mut refresh_button = button::Button::new(x + w - 176, y + 20, 80, 36, "Refresh");
    theme::style_secondary_button(&mut refresh_button);
    let mut delete_button = button::Button::new(x + w - 84, y + 20, 64, 36, "Delete");
    theme::style_secondary_button(&mut delete_button);

    let mut browser = browser::HoldBrowser::new(x + 20, y + 74, w - 40, h - 152, None);
    theme::style_browser(&mut browser);

    // Add status bar for loaded companies
    let mut status_frame = frame::Frame::new(x + 20, y + h - 58, w - 40, 24, "");
    theme::style_toolbar_label(&mut status_frame);

    group.end();

    let company_rows = Rc::new(RefCell::new(Vec::<CompanyRow>::new()));
    let all_companies = Rc::new(RefCell::new(Vec::<CompanyRow>::new()));
    let detail_windows = detail_window::new_store();
    let form_windows = Rc::new(RefCell::new(Vec::<window::Window>::new()));
    let offset = Rc::new(RefCell::new(0u32));
    let total = Rc::new(RefCell::new(0u32));
    let search_term = Rc::new(RefCell::new(String::new()));

    let search_browser = browser.clone();
    let search_rows = company_rows.clone();
    let search_all = all_companies.clone();
    let search_total = total.clone();
    let search_offset = offset.clone();
    let mut search_status = status_frame.clone();
    let search_term_ref = search_term.clone();
    // Search-as-you-type in memory (no DB request)
    search_input.set_callback(move |inp| {
        let search = inp.value();
        *search_term_ref.borrow_mut() = search.clone();
        apply_in_memory_filter(
            &mut search_browser.clone(),
            &search_rows,
            &search_all,
            &search,
            search_total.clone(),
            search_offset.clone(),
            &mut search_status,
        );
    });

    let new_tx = db_tx.clone();
    let form_windows_for_new = form_windows.clone();
    new_button.set_callback(move |_| {
        let form_window = new_company_form::open_new_company_form(new_tx.clone());
        form_windows_for_new.borrow_mut().push(form_window);
    });

    let refresh_tx = db_tx.clone();
    refresh_button.set_callback(move |_| {
        let req = FetchCompaniesReq {
            search: String::new(),
            offset: 0,
            limit: 2_000_000,
        };
        let _ = refresh_tx.send(DbTask::FetchCompanies(req));
    });

    let delete_tx = db_tx.clone();
    let delete_browser = browser.clone();
    let delete_rows = company_rows.clone();

    let mut search_input_for_delete = search_input.clone();
    let refresh_tx_for_delete = db_tx.clone();
    delete_button.set_callback(move |_| {
        let selected = delete_browser.value();
        if selected <= 0 {
            return;
        }
        let idx = (selected - 1) as usize;
        if let Some(company) = delete_rows.borrow().get(idx) {
            let _ = delete_tx.send(DbTask::DeleteCompany(company.id));
            // After deletion, clear search and refresh
            search_input_for_delete.set_value("");
            let req = FetchCompaniesReq {
                search: String::new(),
                offset: 0,
                limit: 2_000_000,
            };
            let _ = refresh_tx_for_delete.send(DbTask::FetchCompanies(req));
        }
    });

    let open_tx = db_tx;
    let open_rows = company_rows.clone();
    let open_windows = detail_windows.clone();
    let open_browser = browser.clone();
    browser.set_callback(move |_| {
        let rows = open_rows.borrow();
        let selected = open_browser.value();
        if !app::event_clicks() {
            return;
        }
        if selected <= 0 {
            return;
        }
        let idx = (selected - 1) as usize;
        let company_opt = rows.get(idx).cloned();
        if let Some(company) = company_opt {
            detail_window::open_detail_window(
                company.id,
                company.name.clone(),
                open_tx.clone(),
                open_windows.clone(),
            );
        }
    });

    CompaniesTabUi {
        status_frame,
        _group: group,
        search_input,
        browser,

        all_companies: all_companies.clone(),
        company_rows: company_rows.clone(),
        detail_windows,
        _form_windows: form_windows,
        total: total.clone(),
        offset: offset.clone(),
        search_term: search_term.clone(),
    }
}

fn apply_in_memory_filter(
    browser: &mut browser::HoldBrowser,
    company_rows: &Rc<RefCell<Vec<CompanyRow>>>,
    all_companies: &Rc<RefCell<Vec<CompanyRow>>>,
    search: &str,
    total: Rc<RefCell<u32>>,
    offset: Rc<RefCell<u32>>,
    status_frame: &mut frame::Frame,
) {
    let search_lower = search.trim().to_lowercase();
    let all = all_companies.borrow();
    let filtered: Vec<CompanyRow> = if search_lower.is_empty() {
        all.clone()
    } else {
        all.iter()
            .filter(|row| row.name.to_lowercase().contains(&search_lower))
            .cloned()
            .collect()
    };

    browser.clear();
    {
        let mut slots = company_rows.borrow_mut();
        slots.clear();
        for row in &filtered {
            browser.add(&format!("  {}", row.name));
            slots.push(row.clone());
        }
    }
    *total.borrow_mut() = filtered.len() as u32;
    *offset.borrow_mut() = filtered.len() as u32;
    status_frame.set_label(&format!(
        "Loaded {} companies ({} total in memory)",
        filtered.len(),
        all.len()
    ));
}

#[allow(dead_code)]
pub fn populate_companies(
    browser: &mut browser::HoldBrowser,
    company_rows: &Rc<RefCell<Vec<CompanyRow>>>,
    rows: &[CompanyRow],
    append: bool,
) {
    if !append {
        browser.clear();
        let mut slots = company_rows.borrow_mut();
        slots.clear();
        for row in rows {
            browser.add(&format!("  {}", row.name));
            slots.push(row.clone());
        }
    } else {
        let mut slots = company_rows.borrow_mut();
        for row in rows {
            browser.add(&format!("  {}", row.name));
            slots.push(row.clone());
        }
    }
}

// Update this function to handle infinite scroll result appending
pub fn handle_companies_result(
    browser: &mut browser::HoldBrowser,
    all_companies: &Rc<RefCell<Vec<CompanyRow>>>,
    company_rows: &Rc<RefCell<Vec<CompanyRow>>>,
    rows: &[CompanyRow],
    search: &str,
    status_frame: &mut frame::Frame,
    total: Rc<RefCell<u32>>,
    offset: Rc<RefCell<u32>>,
) {
    {
        let mut all = all_companies.borrow_mut();
        all.clear();
        all.extend_from_slice(rows);
    }
    apply_in_memory_filter(
        browser,
        company_rows,
        all_companies,
        search,
        total,
        offset,
        status_frame,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::CompanyRow;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn tracer_bullet_loads_companies_in_memory() {
        // Setup: create a fake list of companies
        let companies = vec![
            CompanyRow {
                id: 1,
                name: "Acme Corp".to_string(),
            },
            CompanyRow {
                id: 2,
                name: "Beta LLC".to_string(),
            },
        ];

        let all_companies = Rc::new(RefCell::new(Vec::new()));
        let company_rows = Rc::new(RefCell::new(Vec::new()));
        let mut status_frame = fltk::frame::Frame::default();
        let total = Rc::new(RefCell::new(0));
        let offset = Rc::new(RefCell::new(0));
        let mut browser = fltk::browser::HoldBrowser::default();

        // Act: simulate loading companies into memory
        super::handle_companies_result(
            &mut browser,
            &all_companies,
            &company_rows,
            &companies,
            "",
            &mut status_frame,
            total.clone(),
            offset.clone(),
        );

        // Assert: all companies are loaded and visible in memory
        let loaded = all_companies.borrow();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "Acme Corp");
        assert_eq!(loaded[1].name, "Beta LLC");
    }
}
