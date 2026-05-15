use crate::tasks::{CompanyRow, DbTask, FetchCompaniesReq};
use crate::ui::{detail_window, new_company_form};
use crossbeam::channel::Sender;
use fltk::{app, browser, button, enums::CallbackTrigger, frame, group, input, prelude::*, window};
use std::cell::RefCell;
use std::rc::Rc;

pub struct CompaniesTabUi {
    pub group: group::Group,
    pub search_input: input::Input,
    pub browser: browser::HoldBrowser,
    pub status_frame: frame::Frame,
    pub company_rows: Rc<RefCell<Vec<CompanyRow>>>,
    pub detail_windows: detail_window::DetailWindowStore,
    pub form_windows: Rc<RefCell<Vec<window::Window>>>,
}

pub fn build_companies_tab(x: i32, y: i32, w: i32, h: i32, db_tx: Sender<DbTask>) -> CompaniesTabUi {
    let mut group = group::Group::new(x, y, w, h, "Companie");

    let mut search_input = input::Input::new(x + 90, y + 20, w - 350, 30, "Search:");
    search_input.set_trigger(CallbackTrigger::Changed);

    let mut new_button = button::Button::new(x + w - 240, y + 20, 105, 30, "+ New");
    let mut refresh_button = button::Button::new(x + w - 130, y + 20, 50, 30, "Ref");
    let mut delete_button = button::Button::new(x + w - 75, y + 20, 55, 30, "Del");

    let mut browser = browser::HoldBrowser::new(x + 20, y + 65, w - 40, h - 130, None);
    let status_frame = frame::Frame::new(x + 20, y + h - 55, w - 40, 28, "");

    group.end();

    let company_rows = Rc::new(RefCell::new(Vec::<CompanyRow>::new()));
    let detail_windows = detail_window::new_store();
    let form_windows = Rc::new(RefCell::new(Vec::<window::Window>::new()));

    let search_tx = db_tx.clone();
    search_input.set_callback(move |inp| {
        let req = FetchCompaniesReq {
            search: inp.value(),
            offset: 0,
            limit: 500,
        };
        let _ = search_tx.send(DbTask::FetchCompanies(req));
    });

    let new_tx = db_tx.clone();
    let form_windows_for_new = form_windows.clone();
    new_button.set_callback(move |_| {
        let form_window = new_company_form::open_new_company_form(new_tx.clone());
        form_windows_for_new.borrow_mut().push(form_window);
    });

    let refresh_tx = db_tx.clone();
    let refresh_search = search_input.clone();
    refresh_button.set_callback(move |_| {
        let req = FetchCompaniesReq {
            search: refresh_search.value(),
            offset: 0,
            limit: 500,
        };
        let _ = refresh_tx.send(DbTask::FetchCompanies(req));
    });

    let delete_tx = db_tx.clone();
    let delete_browser = browser.clone();
    let delete_rows = company_rows.clone();
    let mut delete_status = status_frame.clone();
    delete_button.set_callback(move |_| {
        let selected = delete_browser.value();
        if selected <= 0 {
            delete_status.set_label("Select a company before deleting.");
            return;
        }

        let idx = (selected - 1) as usize;
        if let Some(company) = delete_rows.borrow().get(idx) {
            let _ = delete_tx.send(DbTask::DeleteCompany(company.id));
        }
    });

    let open_tx = db_tx;
    let open_rows = company_rows.clone();
    let open_windows = detail_windows.clone();
    let open_browser = browser.clone();
    browser.set_callback(move |_| {
        if !app::event_clicks() {
            return;
        }

        let selected = open_browser.value();
        if selected <= 0 {
            return;
        }

        let idx = (selected - 1) as usize;
        if let Some(company) = open_rows.borrow().get(idx) {
            detail_window::open_detail_window(
                company.id,
                company.name.clone(),
                open_tx.clone(),
                open_windows.clone(),
            );
        }
    });

    CompaniesTabUi {
        group,
        search_input,
        browser,
        status_frame,
        company_rows,
        detail_windows,
        form_windows,
    }
}

pub fn populate_companies(
    browser: &mut browser::HoldBrowser,
    company_rows: &Rc<RefCell<Vec<CompanyRow>>>,
    rows: &[CompanyRow],
) {
    browser.clear();
    let mut slots = company_rows.borrow_mut();
    slots.clear();
    for row in rows {
        browser.add(&row.name);
        slots.push(row.clone());
    }
}
