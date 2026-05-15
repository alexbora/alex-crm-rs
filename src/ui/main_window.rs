use crate::tasks::DbTask;
use crate::ui::companies_tab::{self, CompaniesTabUi};
use crossbeam::channel::Sender;
use fltk::{button, frame, group, input, prelude::*, window};

pub struct MainWindowUi {
    pub window: window::Window,
    pub companies: CompaniesTabUi,
    pub logs_editor: input::MultilineInput,
    pub logs_status: frame::Frame,
    pub status_frame: frame::Frame,
}

pub fn build_main_window(db_tx: Sender<DbTask>) -> MainWindowUi {
    let (screen_w, screen_h) = fltk::app::screen_size();
    let width = 980;
    let height = 700;

    let mut wind = window::Window::new(
        ((screen_w as i32 - width) / 2).max(20),
        ((screen_h as i32 - height) / 2).max(20),
        width,
        height,
        "Alex CRM",
    );

    let mut tabs = group::Tabs::new(10, 10, width - 20, height - 65, None);

    let companies = companies_tab::build_companies_tab(18, 40, width - 36, height - 110, db_tx.clone());

    {
        let mut contacts_group = group::Group::new(18, 40, width - 36, height - 110, "Contacts");
        frame::Frame::new(40, 80, 360, 40, "Contacts tab is ready for backend wiring.");
        contacts_group.end();
    }

    {
        let mut activities_group = group::Group::new(18, 40, width - 36, height - 110, "Activities");
        frame::Frame::new(40, 80, 360, 40, "Activities tab is ready for backend wiring.");
        activities_group.end();
    }

    let (logs_editor, logs_status) = {
        let mut logs_group = group::Group::new(18, 40, width - 36, height - 110, "Logs");
        let logs_editor = input::MultilineInput::new(36, 80, width - 72, height - 240, "Daily Log:");
        let mut save_button = button::Button::new(36, height - 145, 120, 34, "Save Today");
        let mut reload_button = button::Button::new(166, height - 145, 120, 34, "Reload");
        let mut backup_button = button::Button::new(296, height - 145, 140, 34, "Request Backup");
        let logs_status = frame::Frame::new(36, height - 105, width - 72, 28, "");

        let save_tx = db_tx.clone();
        let save_editor = logs_editor.clone();
        save_button.set_callback(move |_| {
            let _ = save_tx.send(DbTask::SaveTodayLog(save_editor.value()));
        });

        let reload_tx = db_tx.clone();
        reload_button.set_callback(move |_| {
            let _ = reload_tx.send(DbTask::FetchTodayLog);
        });

        let backup_tx = db_tx.clone();
        backup_button.set_callback(move |_| {
            let _ = backup_tx.send(DbTask::RequestBackup);
        });

        logs_group.end();
        (logs_editor, logs_status)
    };

    tabs.end();

    let status_frame = frame::Frame::new(16, height - 45, width - 32, 30, "Ready.");
    wind.end();
    wind.make_resizable(true);

    MainWindowUi {
        window: wind,
        companies,
        logs_editor,
        logs_status,
        status_frame,
    }
}
