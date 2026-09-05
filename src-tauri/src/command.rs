use log::{error, info};
use tauri::{State, Webview, Window, command};

use crate::{
    browser::Browser,
    error::{DatabaseError, FetchError, FrameworkError, StateError, TabError},
    history::{HistoryEvent, HistorySnapshotEntry},
    log::QueryLogResponse,
    page::PageToken,
    request::{self, FetchOptions, Response},
    state::BrowserState,
};

#[command]
pub async fn minimize(window: Window) -> Result<(), FrameworkError> {
    window.minimize()?;
    Ok(())
}

#[command]
pub async fn maximize(browser: State<'_, Browser>) -> Result<(), FrameworkError> {
    if let Err(e) = browser.maximize().await {
        error!("最大化失败：{e}");
    }
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn unmaximize(browser: State<'_, Browser>) -> Result<(), FrameworkError> {
    if let Err(e) = browser.unmaximize().await {
        error!("取消最大化失败：{e}");
    }
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub fn close(window: Window) {
    if let Err(e) = window.close() {
        error!("关窗失败：{e}");
    }
}

#[command]
pub async fn start_dragging(
    browser: State<'_, Browser>,
    window: Window,
) -> Result<(), FrameworkError> {
    if let Err(e) = window.start_dragging() {
        error!("开始拖拽失败：{e}");
    }
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn focus(browser: State<'_, Browser>) -> Result<(), StateError> {
    browser.focus().await?;
    Ok(())
}

#[command]
pub async fn blur(browser: State<'_, Browser>) -> Result<(), StateError> {
    browser.blur().await?;
    Ok(())
}

#[command]
pub async fn get_state(browser: State<'_, Browser>) -> Result<BrowserState, StateError> {
    browser.get_state(None).await
}

#[command]
pub async fn search(browser: State<'_, Browser>, keyword: String) -> Result<(), TabError> {
    let Some(url) = browser.parse_keyword(&keyword).await else {
        return Ok(());
    };
    browser.open_tab_by_url(&url, true).await?;
    Ok(())
}

#[command]
pub async fn open_tab(browser: State<'_, Browser>, id: i64) -> Result<(), TabError> {
    browser.open_tab(id).await?;
    Ok(())
}

#[command]
pub async fn back(browser: State<'_, Browser>) -> Result<(), StateError> {
    browser.back().await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn forward(browser: State<'_, Browser>) -> Result<(), StateError> {
    browser.forward().await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn go(browser: State<'_, Browser>, index: usize) -> Result<(), StateError> {
    browser.go(index).await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn reload(browser: State<'_, Browser>) -> Result<(), StateError> {
    browser.reload().await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn incognito(browser: State<'_, Browser>) -> Result<(), TabError> {
    browser.incognito().await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn http_client(browser: State<'_, Browser>) -> Result<(), StateError> {
    browser.http_client().await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command(rename_all = "snake_case")]
pub async fn query_navigation_log(
    browser: State<'_, Browser>,
    keyword: String,
    page_token: PageToken,
) -> Result<QueryLogResponse, DatabaseError> {
    browser.query_navigation_log(keyword, page_token).await
}

#[command]
pub async fn update_star(browser: State<'_, Browser>, id: i64) -> Result<(), DatabaseError> {
    browser.update_star(id).await
}

#[command]
pub async fn content_loaded(
    browser: State<'_, Browser>,
    webview: Webview,
    url: String,
    length: i32,
    icon_url: String,
) -> Result<(), StateError> {
    let label = webview.label();
    info!("{label} webview content loaded {url} {icon_url}");
    browser
        .enqueue_history(
            label,
            HistoryEvent::Load {
                url,
                length: length as usize,
                icon_url,
            },
        )
        .await;
    Ok(())
}

#[command]
pub async fn history_snapshot(
    browser: State<'_, Browser>,
    webview: Webview,
    index: usize,
    entries: Vec<HistorySnapshotEntry>,
) -> Result<(), StateError> {
    let label = webview.label();
    info!(
        "{label} webview history snapshot index={index} entries={}",
        entries.len()
    );
    browser
        .enqueue_history(label, HistoryEvent::Snapshot { index, entries })
        .await;
    Ok(())
}

#[command]
pub async fn fullscreen_changed(
    browser: State<'_, Browser>,
    webview: Webview,
    is_fullscreen: bool,
) -> Result<(), FrameworkError> {
    info!(
        "{} webview fullscreen changed: {is_fullscreen}",
        webview.label()
    );
    browser.fullscreen_changed(is_fullscreen).await?;
    Ok(())
}

#[command]
pub async fn leave_picture_in_picture(
    browser: State<'_, Browser>,
    webview: Webview,
) -> Result<(), FrameworkError> {
    if let Err(e) = browser.leave_picture_in_picture(webview.label()).await {
        error!("退出画中画失败：{e}");
    }
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn focus_link(browser: State<'_, Browser>, url: String) -> Result<(), StateError> {
    browser.focus_link(url).await
}

#[command]
pub async fn blur_link(browser: State<'_, Browser>) -> Result<(), StateError> {
    browser.blur_link().await
}

#[command]
pub async fn click_link(browser: State<'_, Browser>, url: String) -> Result<(), StateError> {
    browser.click_link(url).await
}

#[command]
pub async fn darkreader(browser: State<'_, Browser>) -> Result<(), StateError> {
    browser.darkreader().await
}

#[command]
pub async fn fetch(url: String, options: Option<FetchOptions>) -> Result<Response, FetchError> {
    request::fetch(&url, options).await
}

#[command]
pub async fn close_floating_tab(browser: State<'_, Browser>) -> Result<(), FrameworkError> {
    browser.close_floating_tab().await
}

#[command]
pub async fn promote_floating_tab(browser: State<'_, Browser>) -> Result<(), TabError> {
    browser.promote_floating_tab().await
}
