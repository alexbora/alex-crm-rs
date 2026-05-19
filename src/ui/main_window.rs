use crate::tasks::DbTask;
use crate::ui::companies_tab::{self, CompaniesTabUi};
use crate::ui::theme;
use crossbeam::channel::Sender;
use fltk::{button, frame, group, input, prelude::*, window};

pub struct MainWindowUi {
    pub window: window::Window,
    pub companies: CompaniesTabUi,
    pub logs_editor: input::MultilineInput,
    pub logs_status: frame::Frame,

}

pub fn build_main_window(db_tx: Sender<DbTask>) -> MainWindowUi {
    let (screen_w, screen_h) = fltk::app::screen_size();
    let width = 1040;
    let height = 760;

    let mut wind = window::Window::new(
        ((screen_w as i32 - width) / 2).max(20),
        ((screen_h as i32 - height) / 2).max(20),
        width,
        height,
        "Alex CRM",
    );
    theme::style_window(&mut wind);

    let mut tabs = group::Tabs::new(10, 10, width - 20, height - 65, None);
    theme::style_tabs(&mut tabs);

    let companies =
        companies_tab::build_companies_tab(18, 42, width - 36, height - 116, db_tx.clone());

    {
        let mut contacts_group = group::Group::new(18, 40, width - 36, height - 110, "Contacts");
        theme::style_tab_panel(&mut contacts_group);
        let mut title = frame::Frame::new(40, 82, 220, 26, "Contacts");
        theme::style_section_title(&mut title);
        let mut placeholder = frame::Frame::new(
            40,
            126,
            width - 116,
            64,
            "Contacts tab is ready for backend wiring.",
        );
        theme::style_placeholder_message(&mut placeholder);
        contacts_group.end();
    }

    {
        let mut activities_group =
            group::Group::new(18, 40, width - 36, height - 110, "Activities");
        theme::style_tab_panel(&mut activities_group);
        let mut title = frame::Frame::new(40, 82, 220, 26, "Activities");
        theme::style_section_title(&mut title);
        let mut placeholder = frame::Frame::new(
            40,
            126,
            width - 116,
            64,
            "Activities tab is ready for backend wiring.",
        );
        theme::style_placeholder_message(&mut placeholder);
        activities_group.end();
    }

    let (logs_editor, logs_status) = {
        let mut logs_group = group::Group::new(18, 40, width - 36, height - 110, "Logs");
        theme::style_tab_panel(&mut logs_group);
        let mut logs_title = frame::Frame::new(36, 82, 220, 26, "Daily Log");
        theme::style_section_title(&mut logs_title);
        let mut logs_hint = frame::Frame::new(
            36,
            110,
            width - 72,
            22,
            "Capture what changed today. Entries are saved per day.",
        );
        theme::style_field_hint(&mut logs_hint);
        let mut logs_editor =
            input::MultilineInput::new(36, 142, width - 72, height - 324, "Entry");
        theme::style_multiline_input(&mut logs_editor);
        let mut save_button = button::Button::new(36, height - 155, 136, 36, "Save Today");
        theme::style_primary_button(&mut save_button);
        let mut reload_button = button::Button::new(184, height - 155, 120, 36, "Reload");
        theme::style_secondary_button(&mut reload_button);
        let mut backup_button = button::Button::new(316, height - 155, 160, 36, "Request Backup");
        theme::style_secondary_button(&mut backup_button);
        let mut logs_status = frame::Frame::new(36, height - 109, width - 72, 32, "");
        theme::style_status_frame(&mut logs_status);

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


    wind.end();
    wind.make_resizable(true);

    MainWindowUi {
        window: wind,
        companies,
        logs_editor,
        logs_status,

    }
}
