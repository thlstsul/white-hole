//! 全屏状态机：统一管理 F11 窗口全屏与 webview 元素（视频）全屏。
//!
//! Windows 上 wry 监听 ContainsFullScreenElementChanged，视频全屏的窗口
//! 进入/退出由其原生完成，且随窗口切换保存/还原进入前的摆放；F11 则需
//! 自行驱动 set_fullscreen，并处理与最大化的互斥。两条路径共用
//! is_fullscreen 标志（兼作去重与布局依据），以 f11_entered 区分
//! 当前全屏 episode 的发起方。

use tauri::Window;

use crate::{error::FrameworkError, state::Boolean};

pub struct Fullscreen {
    window: Window,
    /// 自维护全屏标志：与窗口实际状态同步，供布局与 F11 方向判断使用
    is_fullscreen: Boolean,
    /// 当前全屏 episode 是否由 F11 发起（视频可在 F11 全屏中进入，
    /// 此时发起方仍是 F11）
    f11_entered: Boolean,
    /// F11 进入全屏前窗口是否最大化（进入时快照，退出时恢复；
    /// 持久化字段，避免退出时重读 is_maximized 恒为 false）。
    /// 仅 F11 路径读写，用后即清，防止跨 episode 残留
    was_maximized: Boolean,
}

impl Fullscreen {
    pub fn new(window: Window) -> Result<Self, FrameworkError> {
        Ok(Self {
            is_fullscreen: Boolean::from(window.is_fullscreen()?),
            was_maximized: Boolean::default(),
            f11_entered: Boolean::default(),
            window,
        })
    }

    pub async fn is_active(&self) -> bool {
        self.is_fullscreen.get().await
    }

    /// F11 窗口全屏切换：wry 不会为 F11 自动切窗口，需自行处理最大化互斥。
    /// 布局由 set_fullscreen 触发的 WM_SIZE → resize() 收敛，此处不管。
    pub async fn toggle_window(&self, is_fullscreen: bool) -> Result<(), FrameworkError> {
        // 乐观标记：set 返回 false 表示已在目标状态，直接忽略
        // （F11 快速连按第二次读到旧值也不会重复进/退，防方向错乱）
        if !self.is_fullscreen.set(is_fullscreen).await {
            return Ok(());
        }
        let result = if is_fullscreen {
            self.enter().await
        } else {
            self.exit().await
        };
        if let Err(e) = result {
            // 失败回滚标志，避免与窗口实际状态不一致
            self.is_fullscreen.set(!is_fullscreen).await;
            return Err(e);
        }
        Ok(())
    }

    /// webview 元素（视频）全屏上报：窗口切换已由 wry 原生完成（含进入前
    /// 摆放的保存与还原），这里只同步标志并按 episode 发起方补恢复最大化。
    /// 绝不能走 toggle_window 的 unmaximize/set_fullscreen 流程——会与
    /// wry 的原生切换互相打架（窗口被还原成非全屏尺寸）。
    pub async fn webview_changed(&self, is_fullscreen: bool) -> Result<(), FrameworkError> {
        let entered = self.is_fullscreen.set(is_fullscreen).await;
        // 与 toggle_window 一致：窗口操作失败时回滚标志（仅本事件确实翻转过
        // 标志才回滚），避免标志与 wry 窗口实际状态脱节导致后续 F11 方向反转
        let result = if entered {
            if is_fullscreen {
                // 纯视频发起全屏（标志 false→true）：进入前摆放由 wry 保存、
                // 退出时自动还原，不碰 was_maximized；标记本次 episode 非 F11 发起
                self.f11_entered.set(false).await;
                Ok(())
            } else {
                // 视频退出（标志 true→false）：wry 已把窗口还原到 episode
                // 进入前保存的摆放——纯视频时即原始摆放（无需处理）；F11
                // 发起时还原的是 unmaximize 后的摆放，需按 F11 进入前状态
                // 显式恢复最大化
                if self.f11_entered.get().await {
                    self.restore_maximized().await
                } else {
                    Ok(())
                }
            }
        } else {
            // 重复事件（标志已在目标状态）：幂等忽略
            Ok(())
        };
        if let Err(e) = result {
            if entered {
                self.is_fullscreen.set(!is_fullscreen).await;
            }
            return Err(e);
        }
        Ok(())
    }

    /// F11 episode 中视频退出（或 F11 退出）：按进入前状态恢复最大化，
    /// 用后即清，防止快照跨 episode 残留误触发后续 maximize
    async fn restore_maximized(&self) -> Result<(), FrameworkError> {
        if self.was_maximized.get().await {
            self.window.maximize()?;
        }
        self.was_maximized.set(false).await;
        self.f11_entered.set(false).await;
        Ok(())
    }

    /// F11 进入：记录 was_maximized → 若最大化则先 unmaximize()
    /// （清除 WS_MAXIMIZE，解除最大化工作区的客户区钳制，避免全屏底部
    /// 漏出任务栏）→ 再 set_fullscreen(true)，并标记 episode 由 F11 发起。
    async fn enter(&self) -> Result<(), FrameworkError> {
        let was_maximized = self.window.is_maximized()?;
        self.was_maximized.set(was_maximized).await;
        if was_maximized {
            self.window.unmaximize()?;
        }
        self.window.set_fullscreen(true)?;
        self.f11_entered.set(true).await;
        Ok(())
    }

    /// F11 退出：set_fullscreen(false) 后按进入前状态恢复最大化
    async fn exit(&self) -> Result<(), FrameworkError> {
        self.window.set_fullscreen(false)?;
        self.restore_maximized().await
    }
}
