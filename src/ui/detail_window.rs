use crate::tasks::{CompanyDetails, DbTask, UpdateCompanyReq};
use chrono::{Datelike, Local};
use crossbeam::channel::Sender;
use fltk::{button, dialog, input, prelude::*, window};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

pub type DetailWindowStore = Rc<RefCell<HashMap<i64, DetailWindowHandle>>>;

pub struct DetailWindowHandle {
    pub window: window::Window,
    pub name_input: input::Input,
    pub county_input: input::Input,
    pub contact_first_input: input::Input,
    pub contact_last_input: input::Input,
}

pub fn new_store() -> DetailWindowStore {
    Rc::new(RefCell::new(HashMap::new()))
}

pub fn open_detail_window(
    company_id: i64,
    company_name: String,
    db_tx: Sender<DbTask>,
    windows: DetailWindowStore,
) {
    if let Some(existing) = windows.borrow().get(&company_id) {
        let mut win = existing.window.clone();
        win.show();
        let _ = db_tx.send(DbTask::FetchCompanyDetails(company_id));
        return;
    }

    if let Err(err) = open_company_folder(&company_name) {
        dialog::message_default(&format!("Could not open company folder: {err}"));
    }

    let mut wind = window::Window::new(220, 180, 430, 240, "Company Details");

    let name_input = input::Input::new(140, 20, 260, 30, "Name:");
    let county_input = input::Input::new(140, 60, 260, 30, "County:");
    let contact_first_input = input::Input::new(140, 100, 260, 30, "Contact First:");
    let contact_last_input = input::Input::new(140, 140, 260, 30, "Contact Last:");

    let mut save_button = button::Button::new(220, 185, 85, 32, "Save");
    let mut close_button = button::Button::new(315, 185, 85, 32, "Close");

    wind.end();
    wind.make_resizable(true);

    let mut name_for_save = name_input.clone();
    let county_for_save = county_input.clone();
    let contact_first_for_save = contact_first_input.clone();
    let contact_last_for_save = contact_last_input.clone();
    let save_db_tx = db_tx.clone();
    save_button.set_callback(move |_| {
        let new_name = name_for_save.value();
        if new_name.trim().is_empty() {
            dialog::alert_default("Company name is required.");
            return;
        }

        let req = UpdateCompanyReq {
            id: company_id,
            new_name,
            county: county_for_save.value(),
            contact_first: contact_first_for_save.value(),
            contact_last: contact_last_for_save.value(),
        };
        let _ = save_db_tx.send(DbTask::UpdateCompany(req));
    });

    let mut close_wind = wind.clone();
    close_button.set_callback(move |_| {
        close_wind.hide();
    });

    let close_windows = windows.clone();
    wind.set_callback(move |w| {
        w.hide();
        close_windows.borrow_mut().remove(&company_id);
    });

    wind.show();
    windows.borrow_mut().insert(
        company_id,
        DetailWindowHandle {
            window: wind,
            name_input,
            county_input,
            contact_first_input,
            contact_last_input,
        },
    );

    let _ = db_tx.send(DbTask::FetchCompanyDetails(company_id));
}

pub fn apply_company_details(windows: &DetailWindowStore, company_id: i64, details: &CompanyDetails) {
    if let Some(handle) = windows.borrow_mut().get_mut(&company_id) {
        handle.name_input.set_value(&details.name);
        handle.county_input.set_value(&details.county);
        handle.contact_first_input.set_value(&details.contact_first);
        handle.contact_last_input.set_value(&details.contact_last);
        handle.window.redraw();
    }
}

fn open_company_folder(company_name: &str) -> Result<(), String> {
    let now = Local::now();
    let safe_name = sanitize_company_name(company_name);
    let relative_path = PathBuf::from(safe_name)
        .join(now.year().to_string())
        .join(format!("{:02}", now.month()));
    std::fs::create_dir_all(&relative_path).map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(&relative_path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&relative_path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&relative_path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn sanitize_company_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            out.push('_');
        } else {
            out.push(ch);
        }
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        "Company".to_string()
    } else {
        trimmed.to_string()
    }
}
