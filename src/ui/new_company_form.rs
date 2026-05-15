use crate::tasks::{DbTask, InsertCompanyReq};
use crossbeam::channel::Sender;
use fltk::{button, dialog, input, prelude::*, window};

pub fn open_new_company_form(db_tx: Sender<DbTask>) -> window::Window {
    let mut wind = window::Window::new(260, 220, 420, 250, "New Company");

    let name_input = input::Input::new(130, 20, 260, 30, "Name:");
    let county_input = input::Input::new(130, 60, 260, 30, "County:");
    let contact_first_input = input::Input::new(130, 100, 260, 30, "Contact First:");
    let contact_last_input = input::Input::new(130, 140, 260, 30, "Contact Last:");

    let mut save_button = button::Button::new(210, 190, 80, 32, "Save");
    let mut cancel_button = button::Button::new(300, 190, 80, 32, "Cancel");

    wind.end();
    wind.make_resizable(true);

    let mut close_on_save = wind.clone();
    let save_name = name_input.clone();
    let save_county = county_input.clone();
    let save_first = contact_first_input.clone();
    let save_last = contact_last_input.clone();
    save_button.set_callback(move |_| {
        let name = save_name.value();
        if name.trim().is_empty() {
            dialog::alert_default("Company name is required.");
            return;
        }

        let req = InsertCompanyReq {
            name,
            county: save_county.value(),
            contact_first: save_first.value(),
            contact_last: save_last.value(),
        };

        let _ = db_tx.send(DbTask::InsertCompany(req));
        close_on_save.hide();
    });

    let mut close_on_cancel = wind.clone();
    cancel_button.set_callback(move |_| {
        close_on_cancel.hide();
    });

    wind.set_callback(|w| {
        w.hide();
    });

    wind.show();
    wind
}
