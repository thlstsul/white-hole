use serde::{Deserialize, Serialize};
use tauri_sys::core::{invoke, invoke_result};
use time::OffsetDateTime;

pub async fn get_state() -> Result<BrowserState, Error> {
    invoke_result("get_state", &()).await
}

pub async fn search(keyword: String) -> Result<(), Error> {
    invoke_result("search", &SearchRequest { keyword }).await
}

pub async fn open_tab(id: i64) -> Result<(), Error> {
    invoke_result("open_tab", &TouchLogRequest { id }).await
}

pub async fn update_star(id: i64) -> Result<(), Error> {
    invoke_result("update_star", &TouchLogRequest { id }).await
}

pub async fn query_navigation_log(
    keyword: String,
    page_token: PageToken,
) -> Result<QueryLogResponse, Error> {
    invoke_result(
        "query_navigation_log",
        &QueryLogRequest {
            keyword,
            page_token,
        },
    )
    .await
}

pub async fn back() {
    invoke::<()>("back", &()).await;
}

pub async fn forward() {
    invoke::<()>("forward", &()).await;
}

pub async fn reload() {
    invoke::<()>("reload", &()).await;
}

pub async fn incognito() {
    invoke::<()>("incognito", &()).await;
}

pub async fn start_dragging() {
    invoke::<()>("start_dragging", &()).await;
}

pub async fn focus() {
    invoke::<()>("focus", &()).await;
}

#[allow(dead_code)]
pub async fn blur() {
    invoke::<()>("blur", &()).await;
}

pub async fn minimize() {
    invoke::<()>("minimize", &()).await;
}

pub async fn maximize() {
    invoke::<()>("maximize", &()).await;
}

pub async fn unmaximize() {
    invoke::<()>("unmaximize", &()).await;
}

pub async fn close() {
    invoke::<()>("close", &()).await;
}

pub async fn darkreader() {
    invoke::<()>("darkreader", &()).await;
}

#[derive(Debug, Deserialize)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BrowserState {
    pub icon_url: String,
    pub title: String,
    pub url: String,
    pub maximized: bool,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub focus: bool,
    pub incognito: bool,
    pub darkreader: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Deserialize)]
pub struct PageToken {
    pub limit: u32,
    pub offset: u32,
}

impl Default for PageToken {
    fn default() -> Self {
        Self {
            limit: 20,
            offset: 0,
        }
    }
}

#[derive(Clone, Default, PartialEq, Deserialize)]
pub struct NavigationLog {
    pub url: String,
    pub title: String,
    pub icon_url: String,
    pub star: bool,
    pub id: i64,
    pub last_time: Option<OffsetDateTime>,
}

#[derive(Serialize)]
struct SearchRequest {
    keyword: String,
}

#[derive(Serialize)]
struct TouchLogRequest {
    id: i64,
}

#[derive(Serialize)]
struct QueryLogRequest {
    pub keyword: String,
    pub page_token: PageToken,
}

#[derive(Clone, Default, Deserialize)]
pub struct QueryLogResponse {
    pub next_page_token: Option<PageToken>,
    pub logs: Vec<NavigationLog>,
}
