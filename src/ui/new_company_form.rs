use crate::tasks::{DbTask, InsertCompanyReq};
use crate::ui::theme;
use crossbeam::channel::Sender;
use fltk::{button, dialog, frame, input, prelude::*, window};

pub fn open_new_company_form(db_tx: Sender<DbTask>) -> window::Window {
    let mut wind = window::Window::new(260, 220, 480, 322, "New Company");
    theme::style_window(&mut wind);

    let mut title = frame::Frame::new(28, 24, 220, 28, "New Company");
    theme::style_section_title(&mut title);
    let mut help = frame::Frame::new(
        28,
        52,
        420,
        22,
        "Create a company record and optionally add a primary contact.",
    );
    theme::style_field_hint(&mut help);

    let mut name_input = input::Input::new(160, 92, 284, 36, "Name:");
    theme::style_text_input(&mut name_input);
    let mut county_input = input::Input::new(160, 138, 284, 36, "County:");
    theme::style_text_input(&mut county_input);
    let mut contact_first_input = input::Input::new(160, 184, 284, 36, "Contact First:");
    theme::style_text_input(&mut contact_first_input);
    let mut contact_last_input = input::Input::new(160, 230, 284, 36, "Contact Last:");
    theme::style_text_input(&mut contact_last_input);

    let mut save_button = button::Button::new(244, 274, 96, 36, "Save");
    theme::style_primary_button(&mut save_button);
    let mut cancel_button = button::Button::new(348, 274, 96, 36, "Cancel");
    theme::style_secondary_button(&mut cancel_button);

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
