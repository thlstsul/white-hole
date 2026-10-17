use dioxus::prelude::dioxus_stores;
use dioxus::stores::Store;
use dioxus_logger::tracing::error;
use serde::{Deserialize, Serialize};
use std::error;
use std::fmt::{self, Display};
use tauri_sys::core::{invoke as tauri_invoke, invoke_result};
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
    invoke("back").await;
}

pub async fn forward() {
    invoke("forward").await;
}

pub async fn reload() {
    invoke("reload").await;
}

pub async fn incognito() {
    invoke("incognito").await;
}

pub async fn http_client() {
    invoke("http_client").await;
}

pub async fn start_dragging() {
    invoke("start_dragging").await;
}

pub async fn focus() {
    invoke("focus").await;
}

pub async fn minimize() {
    invoke("minimize").await;
}

pub async fn maximize() {
    invoke("maximize").await;
}

pub async fn unmaximize() {
    invoke("unmaximize").await;
}

pub async fn close() {
    tauri_invoke::<()>("close", &()).await;
}

pub async fn darkreader() {
    invoke("darkreader").await;
}

pub async fn fetch(req: HttpRequest) -> Result<HttpResponse, Error> {
    invoke_result("fetch", &req).await
}

#[derive(Debug, Deserialize)]
pub struct Error(String);

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl error::Error for Error {}

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
    pub is_client: bool,
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

#[derive(Clone, Default, PartialEq, Deserialize, Store)]
pub struct NavigationLog {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub icon_url: String,
    pub star: bool,
    pub last_time: Option<OffsetDateTime>,
}

#[derive(Clone, Default, Deserialize)]
pub struct QueryLogResponse {
    pub next_page_token: Option<PageToken>,
    pub logs: Vec<NavigationLog>,
}

#[derive(Debug, PartialEq, Clone, Store, Serialize, Deserialize)]
pub struct HttpHeader {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub url: String,
    pub options: Option<FetchOptions>,
}

/// fetch 函数的可选配置项，支持 JSON 序列化
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchOptions {
    pub method: Option<String>,
    pub headers: Option<Vec<HttpHeader>>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpResponse {
    #[serde(with = "time::serde::iso8601")]
    pub done_date: OffsetDateTime,
    pub status: u16,
    pub headers: Vec<HttpHeader>,
    pub body: Vec<u8>,
    pub elapsed_time: i32,
}

impl HttpRequest {
    pub fn new(url: String, method: String, header: Vec<HttpHeader>, body: String) -> Self {
        let options =
            (!(method.is_empty() && header.is_empty() && body.is_empty())).then(|| FetchOptions {
                method: (!method.is_empty()).then_some(method),
                headers: (!header.is_empty()).then_some(header),
                body: (!body.is_empty()).then_some(body),
            });

        Self { url, options }
    }
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

async fn invoke(command: &str) {
    if let Err(e) = invoke_result::<(), Error>(command, &()).await {
        error!("{command}: {e}");
    }
}
