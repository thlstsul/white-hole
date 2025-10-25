use log::{error, info};
use tauri::{State, Webview, Window, command};

use crate::{
    IsMainView as _,
    browser::Browser,
    error::{DatabaseError, FrameworkError, StateError, TabError},
    log::QueryLogResponse,
    page::PageToken,
    state::BrowserState,
};

#[command]
pub async fn minimize(window: Window, mainview: Webview) -> Result<(), FrameworkError> {
    if !mainview.is_main() {
        return Ok(());
    }

    window.minimize()?;
    Ok(())
}

#[command]
pub async fn maximize(
    browser: State<'_, Browser>,
    mainview: Webview,
) -> Result<(), FrameworkError> {
    if !mainview.is_main() {
        return Ok(());
    }

    if let Err(e) = browser.maximize().await {
        error!("最大化失败：{e}");
    }
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn unmaximize(
    browser: State<'_, Browser>,
    mainview: Webview,
) -> Result<(), FrameworkError> {
    if !mainview.is_main() {
        return Ok(());
    }

    if let Err(e) = browser.unmaximize().await {
        error!("取消最大化失败：{e}");
    }
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub fn close(window: Window, mainview: Webview) {
    if !mainview.is_main() {
        return;
    }
    let _ = window.close().inspect_err(|e| error!("关窗失败：{e}"));
}

#[command]
pub async fn start_dragging(
    browser: State<'_, Browser>,
    window: Window,
    mainview: Webview,
) -> Result<(), FrameworkError> {
    if !mainview.is_main() {
        return Ok(());
    }

    if let Err(e) = window.start_dragging() {
        error!("开始拖拽失败：{e}");
    }
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn focus(browser: State<'_, Browser>, mainview: Webview) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.focus().await?;
    Ok(())
}

#[command]
pub async fn blur(browser: State<'_, Browser>, mainview: Webview) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.blur().await?;
    Ok(())
}

#[command]
pub async fn get_state(
    browser: State<'_, Browser>,
    mainview: Webview,
) -> Result<BrowserState, StateError> {
    if !mainview.is_main() {
        return Err(StateError::NoMainView);
    }

    browser.get_state(None).await
}

#[command]
pub async fn search(
    browser: State<'_, Browser>,
    mainview: Webview,
    keyword: String,
) -> Result<(), TabError> {
    if !mainview.is_main() {
        return Ok(());
    }

    let Some(url) = browser.parse_keyword(&keyword).await else {
        return Ok(());
    };
    browser.open_tab_by_url(&url, true).await?;
    Ok(())
}

#[command]
pub async fn open_tab(
    browser: State<'_, Browser>,
    mainview: Webview,
    id: i64,
) -> Result<(), TabError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.open_tab(id).await?;
    Ok(())
}

#[command]
pub async fn back(browser: State<'_, Browser>, mainview: Webview) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.back().await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn forward(browser: State<'_, Browser>, mainview: Webview) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.forward().await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn go(
    browser: State<'_, Browser>,
    mainview: Webview,
    index: usize,
) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.go(index).await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn reload(browser: State<'_, Browser>, mainview: Webview) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.reload().await?;
    browser.focus_changed().await?;
    Ok(())
}

#[command]
pub async fn incognito(browser: State<'_, Browser>, mainview: Webview) -> Result<(), StateError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.incognito().await?;
    Ok(())
}

#[command(rename_all = "snake_case")]
pub async fn query_navigation_log(
    browser: State<'_, Browser>,
    mainview: Webview,
    keyword: String,
    page_token: PageToken,
) -> Result<QueryLogResponse, DatabaseError> {
    if !mainview.is_main() {
        return Ok(QueryLogResponse::default());
    }

    browser.query_navigation_log(keyword, page_token).await
}

#[command]
pub async fn update_star(
    browser: State<'_, Browser>,
    mainview: Webview,
    id: i64,
) -> Result<(), DatabaseError> {
    if !mainview.is_main() {
        return Ok(());
    }

    browser.update_star(id).await
}

#[command]
pub async fn icon_changed(
    browser: State<'_, Browser>,
    webview: Webview,
    icon_url: String,
) -> Result<(), StateError> {
    if webview.is_main() {
        return Ok(());
    }

    let label = webview.label();
    info!("{label} webview icon changed {icon_url}");
    browser.change_tab_icon(label, icon_url).await?;
    Ok(())
}

#[command]
pub async fn push_history_state(
    browser: State<'_, Browser>,
    webview: Webview,
) -> Result<(), StateError> {
    if webview.is_main() {
        return Ok(());
    }

    info!("{} webview push history state", webview.label());
    browser.push_history_state(webview.label()).await?;
    Ok(())
}

#[command]
pub async fn replace_history_state(
    browser: State<'_, Browser>,
    webview: Webview,
) -> Result<(), StateError> {
    if webview.is_main() {
        return Ok(());
    }

    info!("{} webview replace history state", webview.label());
    browser.replace_history_state(webview.label()).await?;
    Ok(())
}

#[command]
pub async fn pop_history_state(
    browser: State<'_, Browser>,
    webview: Webview,
) -> Result<(), StateError> {
    if webview.is_main() {
        return Ok(());
    }

    info!("{} webview pop history state", webview.label());
    browser.pop_history_state(webview.label()).await?;
    Ok(())
}

#[command]
pub async fn hash_changed(browser: State<'_, Browser>, webview: Webview) -> Result<(), StateError> {
    if webview.is_main() {
        return Ok(());
    }

    info!("{} webview hash changed", webview.label());
    browser.hash_changed(webview.label()).await?;
    Ok(())
}

#[command]
pub async fn fullscreen_changed(
    browser: State<'_, Browser>,
    webview: Webview,
    is_fullscreen: bool,
) -> Result<(), FrameworkError> {
    if webview.is_main() {
        return Ok(());
    }

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
    if webview.is_main() {
        return Ok(());
    }

    if let Err(e) = browser.leave_picture_in_picture(webview.label()).await {
        error!("退出画中画失败：{e}");
    }
    browser.focus_changed().await?;
    Ok(())
}
