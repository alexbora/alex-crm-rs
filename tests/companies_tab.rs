use alex_crm::{tasks::CompanyRow, ui};
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn tracer_bullet_loads_companies_in_memory() {
    // Setup: create a fake list of companies
    let companies = vec![
        CompanyRow { id: 1, name: "Acme Corp".to_string() },
        CompanyRow { id: 2, name: "Beta LLC".to_string() },
    ];

    let all_companies = Rc::new(RefCell::new(Vec::new()));
    let company_rows = Rc::new(RefCell::new(Vec::new()));
    let mut status_frame = fltk::frame::Frame::default();
    let total = Rc::new(RefCell::new(0));
    let offset = Rc::new(RefCell::new(0));
    let mut browser = fltk::browser::HoldBrowser::default();

    // Act: simulate loading companies into memory
    ui::companies_tab::handle_companies_result(
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
