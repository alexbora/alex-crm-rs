use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyRow {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchCompaniesReq {
    pub search: String,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchCompaniesResult {
    pub rows: Vec<CompanyRow>,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsertCompanyReq {
    pub name: String,
    pub county: String,
    pub contact_first: String,
    pub contact_last: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCompanyReq {
    pub id: i64,
    pub new_name: String,
    pub county: String,
    pub contact_first: String,
    pub contact_last: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyDetails {
    pub id: i64,
    pub name: String,
    pub county: String,
    pub contact_first: String,
    pub contact_last: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyLogEntry {
    pub log_date: String,
    pub entry: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub enum UiTask {
    FetchCompaniesResult(Result<FetchCompaniesResult, String>),
    FetchCompanyDetailsResult(i64, Result<CompanyDetails, String>),
    InsertCompanyResult(Result<OperationResult, String>),
    UpdateCompanyResult(Result<OperationResult, String>),
    DeleteCompanyResult(Result<OperationResult, String>),
    FetchTodayLogResult(Result<Option<DailyLogEntry>, String>),
    SaveTodayLogResult(Result<OperationResult, String>),
    BackupStatusResult(Result<OperationResult, String>),
}

#[derive(Debug, Clone)]
pub enum DbTask {
    FetchCompanies(FetchCompaniesReq),
    FetchCompanyDetails(i64),
    InsertCompany(InsertCompanyReq),
    UpdateCompany(UpdateCompanyReq),
    DeleteCompany(i64),
    FetchTodayLog,
    SaveTodayLog(String),
    RequestBackup,
    Shutdown,
}
