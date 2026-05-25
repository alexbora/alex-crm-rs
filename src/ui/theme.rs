use fltk::{
    app, browser, button,
    enums::{Color, FrameType},
    frame, group, input,
    prelude::*,
    window,
};

pub fn install() {
    app::background(0x11, 0x16, 0x1B);
    app::background2(0x19, 0x1F, 0x26);
    app::foreground(0xEB, 0xF1, 0xF5);
    app::set_selection_color(0x2F, 0x8F, 0x60);
    app::set_font_size(14);
    app::set_scrollbar_size(14);
    app::set_visible_focus(true);
    app::set_frame_type2(FrameType::UpBox, FrameType::RFlatBox);
    app::set_frame_type2(FrameType::DownBox, FrameType::RFlatBox);
    app::set_frame_type2(FrameType::GtkUpBox, FrameType::RFlatBox);
    app::set_frame_type2(FrameType::GtkDownBox, FrameType::RFlatBox);
    app::set_frame_border_radius_max(8);
}

pub fn window_background() -> Color {
    Color::from_rgb(0x11, 0x16, 0x1B)
}

fn panel_background() -> Color {
    Color::from_rgb(0x17, 0x1D, 0x24)
}

fn surface_background() -> Color {
    Color::from_rgb(0x1E, 0x25, 0x2D)
}

fn elevated_background() -> Color {
    Color::from_rgb(0x25, 0x2E, 0x38)
}

fn text_color() -> Color {
    Color::from_rgb(0xEB, 0xF1, 0xF5)
}

fn muted_text_color() -> Color {
    Color::from_rgb(0x98, 0xA6, 0xB5)
}

fn accent_color() -> Color {
    Color::from_rgb(0x2F, 0x8F, 0x60)
}

fn accent_pressed_color() -> Color {
    Color::from_rgb(0x25, 0x74, 0x4D)
}

pub fn style_window(window: &mut window::Window) {
    window.set_color(window_background());
}

pub fn style_tabs(tabs: &mut group::Tabs) {
    tabs.set_color(panel_background());
    tabs.set_selection_color(surface_background());
    tabs.set_label_color(text_color());
    tabs.set_frame(FrameType::FlatBox);
}

pub fn style_tab_panel(group: &mut group::Group) {
    group.set_color(panel_background());
    group.set_frame(FrameType::FlatBox);
    group.set_label_color(text_color());
    group.set_label_size(14);
}

pub fn style_text_input(input: &mut input::Input) {
    input.set_color(surface_background());
    input.set_text_color(text_color());
    input.set_selection_color(accent_color());
    input.set_frame(FrameType::BorderBox);
    input.set_label_color(muted_text_color());
    input.set_label_size(13);
    input.set_text_size(14);
}

pub fn style_multiline_input(input: &mut input::MultilineInput) {
    input.set_color(surface_background());
    input.set_text_color(text_color());
    input.set_selection_color(accent_color());
    input.set_frame(FrameType::BorderBox);
    input.set_label_color(muted_text_color());
    input.set_label_size(13);
    input.set_text_size(14);
}

pub fn style_browser(browser: &mut browser::HoldBrowser) {
    browser.set_color(surface_background());
    browser.set_selection_color(accent_color());
    browser.set_frame(FrameType::BorderBox);
    browser.set_text_size(14);
    browser.set_scrollbar_size(14);
}

pub fn style_primary_button(button: &mut button::Button) {
    button.set_color(accent_color());
    button.set_selection_color(accent_pressed_color());
    button.set_label_color(text_color());
    button.set_frame(FrameType::RFlatBox);
    button.set_down_frame(FrameType::RFlatBox);
}

pub fn style_secondary_button(button: &mut button::Button) {
    button.set_color(elevated_background());
    button.set_selection_color(surface_background());
    button.set_label_color(text_color());
    button.set_frame(FrameType::RFlatBox);
    button.set_down_frame(FrameType::RFlatBox);
}

pub fn style_status_frame(frame: &mut frame::Frame) {
    frame.set_color(surface_background());
    frame.set_label_color(muted_text_color());
    frame.set_frame(FrameType::BorderBox);
    frame.set_label_size(13);
}

pub fn style_section_title(frame: &mut frame::Frame) {
    frame.set_label_color(text_color());
    frame.set_label_size(18);
}

pub fn style_placeholder_message(frame: &mut frame::Frame) {
    frame.set_color(surface_background());
    frame.set_label_color(muted_text_color());
    frame.set_frame(FrameType::BorderBox);
    frame.set_label_size(15);
}

pub fn style_field_hint(frame: &mut frame::Frame) {
    frame.set_label_color(muted_text_color());
    frame.set_label_size(13);
}

pub fn style_toolbar_label(frame: &mut frame::Frame) {
    frame.set_label_color(muted_text_color());
    frame.set_label_size(13);
}
