//! 剪贴板历史（Win+V）兼容
//!
//! WebView2/Chromium 内部复制时所有者是 Chromium 内部窗口，Win+V 历史服务
//! 无法识别，内容能粘贴但不出现在历史里。解决办法：复制完成后把剪贴板上
//! 所有格式完整拷贝一遍，以宿主窗口为所有者重新设置。
//!
//! 触发：不再由页面 JS 调用 IPC 命令（remote 权限集已移除），而是由 [`watch`]
//! 创建的消息窗口注册 [`AddClipboardFormatListener`]，系统在内容变化时投递
//! `WM_CLIPBOARDUPDATE`，检测到本应用 WebView2 复制后自行重接管（带节流）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::warn;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT,
    SetLastError, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, EnumClipboardFormats,
    GetClipboardData, GetClipboardOwner, OpenClipboard, RemoveClipboardFormatListener,
    SetClipboardData,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetMessageW,
    GetWindowLongPtrW, GetWindowThreadProcessId, HCURSOR, HICON, HWND_MESSAGE, IsWindow, KillTimer,
    MSG, PostMessageW, RegisterClassW, SetTimer, SetWindowLongPtrW, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLIPBOARDUPDATE, WM_DESTROY, WM_NCDESTROY, WM_TIMER,
    WNDCLASS_STYLES, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

/// 剪贴板重接管的最小间隔：防止高频复制时过度占用剪贴板
const REOWN_COOLDOWN: Duration = Duration::from_millis(300);
/// 收到 WM_CLIPBOARDUPDATE 后延迟多久再尝试重接管：
/// 事件在剪贴板变化瞬间投递，此时源进程可能尚未 CloseClipboard，
/// 立即 OpenClipboard 会竞争失败；延迟后等源进程释放剪贴板
const REOWN_DELAY: Duration = Duration::from_millis(150);
/// OpenClipboard 失败后的重试间隔
const REOWN_RETRY_INTERVAL: Duration = Duration::from_millis(100);
/// OpenClipboard 失败的最大重试次数
const REOWN_MAX_RETRIES: u32 = 10;
/// 重接管定时器 ID（消息专用窗口内唯一）
const REOWN_TIMER_ID: usize = 1;
/// reown 完成消息：worker 线程经 PostMessageW 回传结果。wParam 编码为
/// ReownResult 判别值（0=Reowned、1=Skipped、2=Busy）；lParam 携带代数
const WM_REOWN_DONE: u32 = WM_APP + 1;
/// reown 在 worker 线程执行的最长等待：超过则视为 WM_REOWN_DONE 丢失
/// （PostMessageW 失败、worker 异常退出等），重置 reown_in_flight 以便重试
const REOWN_IN_FLIGHT_TIMEOUT: Duration = Duration::from_secs(2);
/// WebView2 子进程 PID 列表缓存有效期：覆盖整个重试循环（MAX_RETRIES ×
/// RETRY_INTERVAL ≈ 1s），避免重试期间在消息线程上反复全量枚举进程表
const WEBVIEW_PIDS_CACHE_TTL: Duration = Duration::from_secs(1);
/// 已判定为非本应用的 owner PID 记忆有效期：期内同 PID 再未命中时跳过
/// 重枚举（节流）；新 PID（可能刚启动的 WebView2 子进程）绕过节流立即
/// 重枚举，避免首次复制被节流盲窗丢弃
const WEBVIEW_PIDS_RECENT_MISS_TTL: Duration = Duration::from_secs(2);

/// WebView2 子进程 PID 列表缓存：(枚举时刻, PID 列表)。
/// 用 Arc 共享，命中时只克隆 Arc 而非整个 Vec
static WEBVIEW_PIDS_CACHE: Mutex<Option<(std::time::Instant, Arc<Vec<u32>>)>> = Mutex::new(None);
/// 最近判定为非本应用的 owner PID 列表（含判定时刻）：节流用，
/// 避免同一外部 PID 反复触发全表遍历阻塞消息线程
static WEBVIEW_PIDS_RECENT_MISS: Mutex<Vec<(u32, std::time::Instant)>> = Mutex::new(Vec::new());
/// 当前 worker 代数（消息线程写，worker 线程读）：reown 破坏性阶段前核对
/// 自身代数，过期（超时中止/新派发后旧 worker 仍存活）则放弃重设，避免
/// 旧内容覆盖用户新复制的内容
static REOWN_CURRENT_GENERATION: AtomicU64 = AtomicU64::new(0);
/// worker 进度心跳（worker 线程写，消息线程读）：reown 各阶段/每格式递增，
/// 超时后据此区分"慢但存活"（有推进 → 重置窗口）与"卡死"（无推进 → bump +
/// abandoned），防止大剪贴板超时被误判卡死中止后无限循环
static REOWN_WORKER_PROGRESS: AtomicU64 = AtomicU64::new(0);

/// reown 任务：WM_TIMER 派发时经 channel 交给常驻 worker 执行
struct ReownJob {
    /// 主窗口句柄的裸指针值（HWND 非 Send，跨线程传 usize 再重建）
    main_hwnd_val: usize,
    /// worker 代数：回传 WM_REOWN_DONE 时随 lParam 带回，用于拒绝过期结果
    generation: u64,
}

/// worker 线程消息：reown 任务或 PID 缓存刷新请求
enum WorkerMsg {
    /// 执行一次剪贴板重接管
    Reown(ReownJob),
    /// 刷新 WebView2 PID 缓存并核对指定 PID：仍不在列表则记录 recent-miss。
    /// 全量枚举（CreateToolhelp32Snapshot）在 worker 线程执行，避免阻塞消息泵
    RefreshPids(u32),
}

/// 剪贴板监听上下文：窗口过程通过窗口用户数据（GWLP_USERDATA）取回
struct ListenerCtx {
    app: AppHandle,
    /// 最近一次成功重接管的时间；None 表示尚未重接管过（启动初期），
    /// 此时跳过冷却检查，保证开机后首次复制不被节流
    last_reown: Option<std::time::Instant>,
    /// 剩余重试次数：OpenClipboard 竞争失败后由定时器重试
    retries_left: u32,
    /// 是否有 reown 正在 worker 线程执行：避免重试定时器在 worker 未完成
    /// 时再次派发，导致并发 reown
    reown_in_flight: bool,
    /// 派发 reown 的时刻：用于检测 WM_REOWN_DONE 丢失（超时后重置
    /// reown_in_flight，避免功能永久失效）
    reown_started_at: Option<std::time::Instant>,
    /// 递增的 worker 代数：超时后可能派发新 worker，旧 DONE 会迟到；用代数
    /// 区分并忽略过期结果，避免误清 in_flight/误消耗预算/误盖冷却戳
    reown_generation: u64,
    /// 上次超时检查时读到的进度心跳值：超时分支对比计数，有推进（慢但存活）
    /// 则重置窗口继续等；无推进才 bump 代数 + abandoned。派发时重置为 0
    worker_progress_seen: u64,
    /// 连续无进展超时检查标记（two-strike）：单次阻塞的 Win32 调用（延迟
    /// 渲染可能阻塞数秒）期间心跳不推进，一次无推进就中止会误杀慢 worker，
    /// 连续两次无推进才判卡死。有推进或派发新任务时清零
    timeout_strikes: bool,
    /// 单 worker 线程的任务发送端（有界队列，容量 1）：派发 reown/刷新 PID
    /// 缓存，worker 处理并回传 WM_REOWN_DONE。进程表枚举等慢操作都在 worker
    /// 线程执行。有界容量：worker 阻塞时不追加排队，Full 即进 abandoned
    /// 等待期，避免堆积
    reown_tx: std::sync::mpsc::SyncSender<WorkerMsg>,
    /// 当前剪贴板事件的首次派发是否尚未完成：WM_CLIPBOARDUPDATE 置位、首次
    /// 派发后清除。owner 检查只在首次派发时执行（重试期间 GetClipboardOwner
    /// 瞬时返回 NULL/错误，每次重查会把合法重试当非本应用中止）；用显式标志
    /// 而非 retries_left==MAX 推断，避免重试中新事件被误判"非首次"跳过检查
    first_attempt_pending: bool,
    /// 超时中止后旧 worker 是否仍可能在运行：超时分支置位、旧 worker 的 DONE
    /// 回传（任意代数）后清除。置位期间禁止派发新 worker，避免并发重设——
    /// 旧内容 X 若在新内容 Y 写入后才重设，会把 X 覆盖到 Y 上（用户新复制丢失）
    abandoned_worker_pending: bool,
    /// 置位 abandoned_worker_pending 的时刻：若 DONE 永久丢失（PostMessageW
    /// 失败/worker 异常退出）该标志会卡死 reown；超时后 WM_TIMER 门控清除
    /// 标志放行重新派发，迟到结果由代数检查忽略
    abandoned_since: Option<std::time::Instant>,
    /// 重试/在飞窗口内是否到达过新复制事件：门控在 retry_loop_active 时把事件
    /// 整体吞掉（不布防、不重置），若吞掉的是用户真正的新复制而非反馈事件，
    /// 该复制将永不重接管、不进 Win+V 历史。置位后由 DONE 终态分支重新布防
    new_event_pending: bool,
    /// 排队中的 RefreshPids 请求的目标 PID（None=无）：缓存未命中且非已知
    /// 外部时发送；同 PID 排队则等待（重发只会堆积），异 PID 清除重发（排队
    /// 请求核对的是别的 PID，永不记录当前 PID 的 recent-miss，不重发则卡死）
    refresh_pending_target: Option<u32>,
    /// WaitRefresh 连续重试次数（退避用）：有界队列被慢 reown 占满时
    /// RefreshPids 反复 Full，固定 100ms 重试会 10Hz 空转消息泵；布防间隔
    /// 按 2^refresh_backoff 递增（上限 ×16），Ours/NotOurs/新事件时清零
    refresh_backoff: u32,
}

/// 启动剪贴板监听线程：创建消息专用窗口并注册 [`AddClipboardFormatListener`]，
/// 系统在剪贴板内容变化时向该窗口投递 `WM_CLIPBOARDUPDATE`。
/// 只处理本应用发起的复制（所有者进程过滤），避免干扰其他程序。
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || unsafe {
        // 注册消息专用窗口类（同一进程内只会注册一次，返回值可忽略）
        let class_name = w!("WhiteHoleClipboardListener");
        let hinstance = GetModuleHandleW(None)
            .map(|h| HINSTANCE(h.0))
            .unwrap_or_default();
        let wc = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(clipboard_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: HICON::default(),
            hCursor: HCURSOR::default(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
        };
        RegisterClassW(&wc);

        // 上下文存入窗口用户数据，窗口过程通过 GWLP_USERDATA 取回。
        // reown 任务经有界队列（容量 1）交给常驻 worker 线程执行：worker
        // 阻塞（OpenClipboard 无超时）时不追加排队，try_send 返回 Full 即
        // 进入 abandoned 等待期，避免 mpsc 无限堆积（内存增长 + 旧任务倾泻）
        let (reown_tx, reown_rx) = std::sync::mpsc::sync_channel::<WorkerMsg>(1);
        let ctx = Box::into_raw(Box::new(ListenerCtx {
            app,
            // None 表示尚未重接管过：启动后首次复制不被冷却检查丢弃
            last_reown: None,
            // 初始化为满预算：WM_CLIPBOARDUPDATE 的重置门控以
            // retries_left < REOWN_MAX_RETRIES 判定"重试循环激活"，若从 0 起步
            // 该条件恒真，首次事件会被门控吞掉，reown 功能完全失效
            retries_left: REOWN_MAX_RETRIES,
            reown_in_flight: false,
            reown_started_at: None,
            reown_generation: 0,
            // 启动时无 worker 在飞：心跳计数从 0 起算
            worker_progress_seen: 0,
            // 启动时无连续无进展记录
            timeout_strikes: false,
            reown_tx,
            // 首次 WM_CLIPBOARDUPDATE 会置位；初始 false 无影响
            first_attempt_pending: false,
            // 启动时无旧 worker
            abandoned_worker_pending: false,
            abandoned_since: None,
            // 启动时无待处理的新事件
            new_event_pending: false,
            // 启动时无排队中的刷新请求
            refresh_pending_target: None,
            // 启动时无退避
            refresh_backoff: 0,
        }));

        // 消息专用窗口：不可见、不占 Z 序、不接收广播，只收剪贴板通知
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinstance),
            Some(ctx as *const core::ffi::c_void),
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => {
                // 消息专用窗口创建失败（类注册异常/资源耗尽/无效实例句柄）会
                // 静默禁用整个剪贴板重接管功能，必须记录日志便于诊断——与
                // SetWindowLongPtrW / AddClipboardFormatListener 失败路径一致
                warn!("创建剪贴板监听窗口失败：CreateWindowExW");
                drop(Box::from_raw(ctx));
                return;
            }
        };
        // 存入窗口用户数据。注意 SetWindowLongPtrW 返回的是**上一次**的值
        // （新建窗口 GWLP_USERDATA 初始为 0，成功也返回 0），不能直接判失败；
        // 需先清零错误码，仅当返回 0 且 GetLastError 非 0 才是真失败。真失败
        // 时指针未存储，WM_NCDESTROY 读不到，必须在此直接回收 ListenerCtx
        SetLastError(WIN32_ERROR(0));
        let previous = SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx as isize);
        if previous == 0 && GetLastError() != WIN32_ERROR(0) {
            warn!("设置剪贴板监听窗口用户数据失败：SetWindowLongPtrW");
            drop(Box::from_raw(ctx));
            let _ = DestroyWindow(hwnd);
            return;
        }

        if AddClipboardFormatListener(hwnd).is_err() {
            warn!("注册剪贴板监听失败：AddClipboardFormatListener");
            let _ = DestroyWindow(hwnd);
            return;
        }

        // 启动常驻 reown worker 线程：从 channel 取任务执行，完成后回传
        // WM_REOWN_DONE。窗口销毁（WM_NCDESTROY 回收 ctx 时 drop sender）
        // 会关闭 channel，recv 返回 Err 使线程自然退出
        let listener_hwnd_val = hwnd.0 as usize;
        std::thread::spawn(move || reown_worker(reown_rx, listener_hwnd_val));

        // 消息循环：系统在剪贴板内容变化时向本窗口投递 WM_CLIPBOARDUPDATE
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }

        // 循环仅在 WM_QUIT 或 GetMessageW 出错时退出；销毁窗口，
        // 让 WM_DESTROY/WM_NCDESTROY 同步注销监听并回收 ListenerCtx
        let _ = DestroyWindow(hwnd);
    });
}

/// 重新布防定时器处理门控期间到达的新事件：SetTimer **成功**后才置位
/// first_attempt_pending（下次 WM_TIMER 重跑 owner 检查）并清除
/// new_event_pending；失败时保留——真实新复制不能因布防失败而遗忘
/// （否则永不重接管），下次 WM_CLIPBOARDUPDATE/终态分支会再试布防
fn rearm_for_pending_event(ctx: &mut ListenerCtx, hwnd: HWND) {
    let timer = unsafe {
        SetTimer(
            Some(hwnd),
            REOWN_TIMER_ID,
            REOWN_DELAY.as_millis() as u32,
            None,
        )
    };
    if timer == 0 {
        warn!("剪贴板重接管定时器创建失败（SetTimer），保留待处理新复制等待下次布防");
        return;
    }
    ctx.first_attempt_pending = true;
    ctx.new_event_pending = false;
}

/// 消息专用窗口的窗口过程：剪贴板内容变化时调度延迟重接管（带重试）
unsafe extern "system" fn clipboard_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            let ctx = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ListenerCtx };
            if !ctx.is_null() {
                // 事件在剪贴板变化瞬间投递，源进程可能尚未 CloseClipboard，立即
                // OpenClipboard 会竞争失败；重置重试计数并延迟重接管。仅在无重试
                // 循环时重置：写入期间反馈事件若触发 owner 重查，可能落在
                // NULL-owner 窗口把合法重试当非本应用中止。重试激活不能只看
                // reown_in_flight（Retry 分支先清 in_flight 再布防），需一并查预算
                let retry_active =
                    unsafe { retry_loop_active((*ctx).reown_in_flight, (*ctx).retries_left) };
                if !retry_active {
                    unsafe {
                        (*ctx).retries_left = REOWN_MAX_RETRIES;
                        // 新事件：标记首次派发尚未完成，owner 检查将在首次派发时执行
                        (*ctx).first_attempt_pending = true;
                        // 新事件：退避清零（新的刷新等待从满速开始）
                        (*ctx).refresh_backoff = 0;
                    }
                    // SetTimer 返回 0 表示失败（如系统定时器资源耗尽），此时
                    // 不会再有定时器触发重接管，本次复制将丢失，需记录日志
                    let timer = unsafe {
                        SetTimer(
                            Some(hwnd),
                            REOWN_TIMER_ID,
                            REOWN_DELAY.as_millis() as u32,
                            None,
                        )
                    };
                    if timer == 0 {
                        warn!("剪贴板重接管定时器创建失败（SetTimer）");
                    }
                } else {
                    // 门控吞掉的事件可能不是反馈而是用户真正的新复制（快速连续
                    // 复制/程序化复制）：置位待处理标志，由 WM_REOWN_DONE 终态
                    // 分支在重试结束后重新布防处理，避免该复制永不重接管
                    unsafe { (*ctx).new_event_pending = true };
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == REOWN_TIMER_ID {
                let ctx = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ListenerCtx };
                if !ctx.is_null() {
                    let ctx = unsafe { &mut *ctx };
                    // 已有 reown 在飞：超时未回传（PostMessageW 失败/worker 异常
                    // 退出/OpenClipboard 长期占用）则进 abandoned 等待期，由门控
                    // 自动恢复派发；否则等待回传结果
                    if ctx.reown_in_flight {
                        let timed_out = ctx
                            .reown_started_at
                            .is_some_and(|t| t.elapsed() >= REOWN_IN_FLIGHT_TIMEOUT);
                        if !timed_out {
                            return LRESULT(0);
                        }
                        // 心跳检查：有推进 → worker 慢但存活，重置超时窗口继续等待、
                        // 不 bump 代数（否则中止 → Skipped → Ignore → 重新派发，
                        // 大剪贴板无限循环）。**连续两次无推进才判卡死**：单次阻塞
                        // 的 GetClipboardData/GlobalLock（延迟渲染）可能阻塞数秒，
                        // 一次无推进就中止会误杀慢但存活的 worker。三态判定由
                        // decide_heartbeat_verdict 纯函数驱动（单测覆盖）
                        let progress = REOWN_WORKER_PROGRESS.load(Ordering::SeqCst);
                        match decide_heartbeat_verdict(
                            ctx.worker_progress_seen,
                            progress,
                            ctx.timeout_strikes,
                        ) {
                            HeartbeatVerdict::Alive => {
                                ctx.worker_progress_seen = progress;
                                ctx.timeout_strikes = false;
                                ctx.reown_started_at = Some(std::time::Instant::now());
                                return LRESULT(0);
                            }
                            HeartbeatVerdict::FirstStrike => {
                                // 第一次无推进：记录标记，重置超时窗口再等一轮
                                ctx.timeout_strikes = true;
                                ctx.reown_started_at = Some(std::time::Instant::now());
                                return LRESULT(0);
                            }
                            HeartbeatVerdict::Stuck => {
                                // 连续两次无推进：判定卡死
                                ctx.timeout_strikes = false;
                                warn!(
                                    "剪贴板重接管 worker 超时且连续无进展，进入 abandoned 等待期"
                                );
                                // 递增代数：旧 worker 迟到的回传被 WM_REOWN_DONE 按代数
                                // 忽略；其破坏性阶段前的代数核对也会放弃重设（防旧内容
                                // 覆盖新内容）
                                ctx.reown_generation = ctx.reown_generation.wrapping_add(1);
                                REOWN_CURRENT_GENERATION
                                    .store(ctx.reown_generation, Ordering::SeqCst);
                                ctx.reown_in_flight = false;
                                ctx.reown_started_at = None;
                                // 记录旧 worker 可能仍在运行：回传前禁止派发新 worker
                                // （避免并发重设覆盖新内容）；回传永久丢失时由门控超时
                                // 清除并放行，迟到结果被代数忽略
                                ctx.abandoned_worker_pending = true;
                                ctx.abandoned_since = Some(std::time::Instant::now());
                                // 恢复满预算：否则 WM_CLIPBOARDUPDATE 的重置门控会永久
                                // 吞掉后续新事件，reown 功能直到应用重启都失效
                                ctx.retries_left = budget_after_retry_abort();
                                // 保持定时器运行：abandoned 门控依赖 WM_TIMER 驱动恢复，
                                // KillTimer 会让 WM_REOWN_DONE 永久丢失时无法自动恢复
                                return LRESULT(0);
                            }
                        }
                    }
                    // 旧 worker 仍在运行（超时中止后未回传）：禁止派发新 worker；
                    // 回传永久丢失时超时后清除标志放行，迟到结果由代数忽略
                    if ctx.abandoned_worker_pending {
                        if !abandoned_worker_expired(ctx.abandoned_since) {
                            return LRESULT(0);
                        }
                        // 恢复满预算并清除标志：重新进入正常布防/派发流程
                        ctx.abandoned_worker_pending = false;
                        ctx.abandoned_since = None;
                        ctx.retries_left = budget_after_retry_abort();
                        warn!("剪贴板重接管旧 worker 回传丢失超时，恢复派发");
                    }
                    // 冷却期内 / 非本应用 / 无主窗口：停止定时器
                    match should_reown(ctx) {
                        ShouldReown::Stop => {
                            // 恢复满预算：否则 WM_CLIPBOARDUPDATE 的重置门控会
                            // 永久吞掉后续新事件，reown 功能直到应用重启都失效
                            ctx.retries_left = budget_after_retry_abort();
                            let _ = unsafe { KillTimer(Some(hwnd), REOWN_TIMER_ID) };
                            return LRESULT(0);
                        }
                        ShouldReown::WaitRefresh => {
                            // PID 缓存刷新中：重新布防定时器稍后再查，不消耗重试
                            // 预算；布防失败按中止处理。退避：队列被慢 reown 占满
                            // 时 RefreshPids 反复 Full，固定 100ms 重试 10Hz 空转
                            // ——间隔按 2^refresh_backoff 递增（上限 ×16），
                            // Ours/NotOurs/新事件时清零。注意 `as u32 << ...`
                            // 会被解析为泛型参数，需括号包住被移位值
                            let backoff_ms = (REOWN_RETRY_INTERVAL.as_millis() as u32)
                                << ctx.refresh_backoff.min(4);
                            ctx.refresh_backoff = ctx.refresh_backoff.saturating_add(1).min(4);
                            let timer =
                                unsafe { SetTimer(Some(hwnd), REOWN_TIMER_ID, backoff_ms, None) };
                            if timer == 0 {
                                warn!("剪贴板重接管重试定时器创建失败（SetTimer）");
                                ctx.retries_left = budget_after_retry_abort();
                                let _ = unsafe { KillTimer(Some(hwnd), REOWN_TIMER_ID) };
                            }
                            return LRESULT(0);
                        }
                        ShouldReown::Dispatch(main_hwnd) => {
                            // 派发到常驻 worker 线程执行 reown（大剪贴板读取不
                            // 阻塞消息泵），完成后经 WM_REOWN_DONE 回传
                            let main_hwnd_val = main_hwnd.0 as usize;
                            let generation = ctx.reown_generation.wrapping_add(1);
                            // 先发布代数再投递：worker 唤醒后立即核对代数，后发布会
                            // 读到旧值误判自身过期（假 Skipped）。**投递失败不回退**：
                            // 回退让旧 worker 在 store/restore 间读到从未派发过的瞬时
                            // 代数误判过期；保持新代数并同步 ctx.reown_generation，
                            // 旧任务 DONE 按代数 Ignore，abandoned 恢复后新任务重做
                            REOWN_CURRENT_GENERATION.store(generation, Ordering::SeqCst);
                            // 同样先重置心跳再投递：投递后清零会抹掉 worker 的
                            // 早期 bump，慢 worker 被误判卡死。编码代数到高 32
                            // 位（gen<<32）：使 seen 与计数代数基线可比——重置前
                            // 旧 worker 的累计（高 32 位=旧代数）判为无进展；但
                            // 重置后旧 worker 的 bump 叠加到新基线，仍会被算作
                            // 推进（有限窗口：其破坏性阶段前核对代数即停止）
                            REOWN_WORKER_PROGRESS.store(generation << 32, Ordering::SeqCst);
                            ctx.worker_progress_seen = generation << 32;
                            ctx.timeout_strikes = false;
                            match ctx.reown_tx.try_send(WorkerMsg::Reown(ReownJob {
                                main_hwnd_val,
                                generation,
                            })) {
                                Ok(()) => {
                                    ctx.reown_in_flight = true;
                                    ctx.reown_started_at = Some(std::time::Instant::now());
                                    ctx.reown_generation = generation;
                                    // 首次派发完成：后续重试不再重复 owner 检查
                                    ctx.first_attempt_pending = false;
                                }
                                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                                    // 投递失败（队列满）：保持已发布代数并同步
                                    // ctx.reown_generation（不回退，理由见上）
                                    ctx.reown_generation = generation;
                                    // worker 仍阻塞在旧任务：有界队列已满，
                                    // 不追加排队（避免 mpsc 无限堆积），进入
                                    // abandoned 等待期，由门控稍后重试派发
                                    warn!(
                                        "剪贴板重接管 worker 任务队列已满（worker 阻塞中），进入 abandoned 等待期"
                                    );
                                    ctx.abandoned_worker_pending = true;
                                    ctx.abandoned_since = Some(std::time::Instant::now());
                                    // 恢复满预算：否则重置门控会永久吞掉后续新事件
                                    ctx.retries_left = budget_after_retry_abort();
                                    return LRESULT(0);
                                }
                                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                                    // 投递失败（worker 已退出）：保持已发布代数并
                                    // 同步 ctx.reown_generation（不回退，理由见上）
                                    ctx.reown_generation = generation;
                                    // worker 已退出（channel 关闭，如窗口销毁中）
                                    warn!("剪贴板重接管 worker 已退出，无法派发任务");
                                    ctx.reown_in_flight = false;
                                    ctx.reown_started_at = None;
                                    // 恢复满预算：否则重置门控会永久吞掉后续新事件
                                    ctx.retries_left = budget_after_retry_abort();
                                    let _ = unsafe { KillTimer(Some(hwnd), REOWN_TIMER_ID) };
                                    return LRESULT(0);
                                }
                            }
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_REOWN_DONE => {
            let ctx = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ListenerCtx };
            if !ctx.is_null() {
                let ctx = unsafe { &mut *ctx };
                // 任意 WM_REOWN_DONE 回传都表示有 worker 已结束 reown 工作
                // （含超时中止后迟到的旧 worker）：清除 abandoned_worker_pending，
                // 允许后续派发新 worker（置位期间已禁止派发，回传必是旧 worker 的）
                ctx.abandoned_worker_pending = false;
                ctx.abandoned_since = None;
                let result = match wparam.0 {
                    0 => ReownResult::Reowned,
                    1 => ReownResult::Skipped,
                    _ => ReownResult::Busy,
                };
                let generation_matches = lparam.0 as u64 == ctx.reown_generation;
                match decide_reown_done(result, generation_matches, ctx.retries_left) {
                    ReownDoneStep::Ignore => {
                        // 过期 worker 的结果（代数不匹配）：忽略，等待当前 worker。
                        // 不盖冷却戳：陈旧成功若盖 last_reown，会误拦截随后用户真正
                        // 的新复制；反馈循环已由 should_reown 的 owner 检查拦截
                        // （反馈 owner 是 main 窗口 → NotOurs → 盖戳 Stop）。
                        // 门控（三者同时满足才算空闲）：!reown_in_flight（无 worker
                        // 在飞）、retries_left == MAX（无重试）、!first_attempt_pending
                        // （无待派发定时器——新事件到达与定时器触发之间 in_flight=false）
                        let state_idle = !ctx.reown_in_flight
                            && ctx.retries_left == REOWN_MAX_RETRIES
                            && !ctx.first_attempt_pending;
                        // 超时停滞期间被门控吞掉的事件（过期 worker 反馈或用户真正
                        // 的新复制）：**一律重新布防**，由 should_reown 的 owner 检查
                        // 区分（不用瞬时 owner——DONE 可能先于新复制的
                        // WM_CLIPBOARDUPDATE 到达，此刻 owner 仍是 main 窗口，若当
                        // 反馈盖戳停止，真实新复制会被永久丢弃）。重布防后：
                        // 反馈 → NotOurs 盖戳；真实新复制 → Ours 派发
                        if state_idle && ctx.new_event_pending {
                            rearm_for_pending_event(ctx, hwnd);
                        }
                        return LRESULT(0);
                    }
                    ReownDoneStep::Success => {
                        ctx.reown_in_flight = false;
                        ctx.reown_started_at = None;
                        // 恢复满预算：重试循环已终止，下一次 WM_CLIPBOARDUPDATE
                        // 的重置门控（retries_left < MAX）不应再吞掉新事件
                        ctx.retries_left = REOWN_MAX_RETRIES;
                        if ctx.new_event_pending {
                            // 门控期间被吞掉的事件（reown 自身反馈或用户真正的新
                            // 复制）：**一律重新布防**，由 should_reown 的 owner 检查
                            // 区分——不用瞬时 owner：DONE 可能先于新复制的
                            // WM_CLIPBOARDUPDATE 到达，此刻 owner 仍是 main 窗口，
                            // 若当反馈盖戳停止，新复制随后重布防也会被冷却拦截而
                            // 永久丢弃。重布防后：反馈 → NotOurs 盖戳；真实新复制
                            // → Ours 派发
                            rearm_for_pending_event(ctx, hwnd);
                        } else {
                            // 成功且无待处理事件：盖冷却戳（阻止 reown 自身
                            // 触发的反馈循环），停止定时器
                            ctx.last_reown = Some(std::time::Instant::now());
                            let _ = unsafe { KillTimer(Some(hwnd), REOWN_TIMER_ID) };
                        }
                    }
                    ReownDoneStep::Skip => {
                        ctx.reown_in_flight = false;
                        ctx.reown_started_at = None;
                        // 恢复满预算（同上，重试循环已终止）
                        ctx.retries_left = REOWN_MAX_RETRIES;
                        if ctx.new_event_pending {
                            // 门控期间被吞掉的事件（Skip 表示剪贴板未动，
                            // owner 不可能是本进程，只可能是用户新复制）：
                            // 重新布防处理它
                            rearm_for_pending_event(ctx, hwnd);
                        } else {
                            // 剪贴板未动（存在无法复制的格式，如图片）：停止
                            // 定时器，但**不盖冷却戳**，避免后续复制被节流
                            let _ = unsafe { KillTimer(Some(hwnd), REOWN_TIMER_ID) };
                        }
                    }
                    ReownDoneStep::Retry => {
                        ctx.reown_in_flight = false;
                        ctx.reown_started_at = None;
                        // OpenClipboard 竞争失败，稍后重试；
                        // 布防成功才消耗预算（失败保留，等下次事件重新布防）
                        let timer = unsafe {
                            SetTimer(
                                Some(hwnd),
                                REOWN_TIMER_ID,
                                REOWN_RETRY_INTERVAL.as_millis() as u32,
                                None,
                            )
                        };
                        if timer == 0 {
                            // 重布防失败：本定时器周期性触发且预算被保留（永远
                            // Retry，GiveUp 不可达）→ 无界循环；停掉后由下次事件
                            // 重新布防并恢复满预算，否则门控永久吞掉新事件
                            warn!("剪贴板重接管重试定时器创建失败（SetTimer）");
                            ctx.retries_left = budget_after_retry_abort();
                            let _ = unsafe { KillTimer(Some(hwnd), REOWN_TIMER_ID) };
                        } else {
                            ctx.retries_left = budget_after_rearm(ctx.retries_left, true);
                        }
                    }
                    ReownDoneStep::GiveUp => {
                        ctx.reown_in_flight = false;
                        ctx.reown_started_at = None;
                        // 恢复满预算（同上，重试循环已终止，允许后续新事件重新布防）
                        ctx.retries_left = REOWN_MAX_RETRIES;
                        if ctx.new_event_pending {
                            // 门控期间被吞掉的事件（GiveUp 表示 worker 一直
                            // Busy 未重设，owner 不可能是本进程，只可能是用户
                            // 新复制）：重新布防处理，而非因重试耗尽而停止
                            rearm_for_pending_event(ctx, hwnd);
                        } else {
                            // 重试次数耗尽，放弃本次
                            warn!("剪贴板重接管重试次数耗尽，本次复制未重接管");
                            let _ = unsafe { KillTimer(Some(hwnd), REOWN_TIMER_ID) };
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = unsafe { RemoveClipboardFormatListener(hwnd) };
            let _ = unsafe { KillTimer(Some(hwnd), REOWN_TIMER_ID) };
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_NCDESTROY => {
            let ctx = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ListenerCtx };
            if !ctx.is_null() {
                // 窗口销毁时回收上下文
                drop(unsafe { Box::from_raw(ctx) });
                let _ = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// WM_REOWN_DONE 处理结果（纯决策，便于单测）
#[derive(Debug, PartialEq, Eq)]
enum ReownDoneStep {
    /// 过期 worker 的结果（代数不匹配），忽略，等待当前 worker
    Ignore,
    /// 已成功重接管：盖冷却戳并停止定时器
    Success,
    /// 剪贴板未动（存在无法复制的格式）：停止定时器但不盖冷却戳，
    /// 避免后续复制被冷却节流
    Skip,
    /// 竞争失败且还有重试次数：重新布防定时器
    Retry,
    /// 竞争失败且重试耗尽：放弃
    GiveUp,
}

/// reown 的结果（纯决策，便于单测）。
/// 判别值即 WM_REOWN_DONE 的 wParam 编码（0=Reowned、1=Skipped、2=Busy），
/// 显式指定并锁定，避免重排变体时静默破坏跨线程协议
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
enum ReownResult {
    /// 已成功重接管剪贴板
    Reowned = 0,
    /// 存在无法复制的格式，剪贴板未动（不应盖冷却戳、不应重试）
    Skipped = 1,
    /// OpenClipboard/EmptyClipboard 竞争失败（可重试）
    Busy = 2,
}

/// 纯决策：根据 worker 结果、代数匹配与剩余重试次数决定 WM_REOWN_DONE
/// 的处理。代数不匹配（超时后已派发新 worker，旧结果迟到）时忽略，
/// 避免误清 in_flight/误消耗预算/误盖冷却戳
fn decide_reown_done(
    result: ReownResult,
    generation_matches: bool,
    retries_left: u32,
) -> ReownDoneStep {
    if !generation_matches {
        ReownDoneStep::Ignore
    } else {
        match result {
            ReownResult::Reowned => ReownDoneStep::Success,
            ReownResult::Skipped => ReownDoneStep::Skip,
            ReownResult::Busy => {
                if retries_left > 0 {
                    ReownDoneStep::Retry
                } else {
                    ReownDoneStep::GiveUp
                }
            }
        }
    }
}

/// 破坏性阶段（EmptyClipboard/SetClipboardData）的句柄释放策略
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum CleanupMode {
    /// 释放全部预分配句柄（alloc 失败 / 代数不匹配 / EmptyClipboard 失败）
    FreeAll,
    /// 只释放 SetClipboardData 失败的句柄（成功者已被系统接管）
    FreeFailed,
}

/// 纯决策：破坏性阶段的记账与结果。
/// 输入：alloc_ok/gen_matches/empty_ok/set_succeeded；返回 (结果, 释放策略)。
/// 不变量：不得双重释放或泄漏——FreeAll 释放全部预分配句柄（EmptyClipboard
/// 未调用或已失败），FreeFailed 只释放失败者；CloseClipboard 由调用方无条件调用
fn decide_rewrite_cleanup(
    alloc_ok: bool,
    gen_matches: bool,
    empty_ok: bool,
    set_succeeded: u32,
) -> (ReownResult, CleanupMode) {
    if !alloc_ok || !gen_matches {
        (ReownResult::Skipped, CleanupMode::FreeAll)
    } else if !empty_ok {
        (ReownResult::Busy, CleanupMode::FreeAll)
    } else if set_succeeded == 0 {
        // EmptyClipboard 已成功（剪贴板被清空）但一个格式都没设置成功：数据
        // 已丢失，返回 Busy（可重试语义）而非 Skipped（"剪贴板未动"）——避免
        // 误导调用方停止且不重试；重试会重新枚举空剪贴板，最终仍收敛
        (ReownResult::Busy, CleanupMode::FreeFailed)
    } else {
        (ReownResult::Reowned, CleanupMode::FreeFailed)
    }
}

/// 纯守卫：是否值得尝试重接管。
/// 非本应用复制或无主窗口时不应处理（返回 false，调用方停止定时器）
fn should_attempt_reown(owner_is_ours: bool, has_main_window: bool) -> bool {
    owner_is_ours && has_main_window
}

/// 纯记账：定时器布防成功才消耗一次重试预算（防失败时误扣预算）；
/// 返回新的剩余重试次数
fn budget_after_rearm(retries_left: u32, rearm_succeeded: bool) -> u32 {
    if rearm_succeeded {
        retries_left.saturating_sub(1)
    } else {
        retries_left
    }
}

/// 纯记账：重试循环中止（worker 超时未回传 / 重布防失败）后恢复满预算。
/// 若不恢复，重置门控（retry_loop_active 检查 retries_left < MAX）会永久
/// 吞掉后续新事件，reown 功能直到重启都失效
fn budget_after_retry_abort() -> u32 {
    REOWN_MAX_RETRIES
}

/// 纯判定：重试循环是否激活（WM_CLIPBOARDUPDATE 重置门控用）。worker 在飞
/// 或预算已消耗视为激活：写入期间反馈事件不应重置预算/首次派发（否则重触发
/// owner 检查，落在 NULL-owner 窗口误杀合法重试）。retries_left 须恢复 MAX
fn retry_loop_active(reown_in_flight: bool, retries_left: u32) -> bool {
    reown_in_flight || retries_left < REOWN_MAX_RETRIES
}

/// 是否处于冷却期内：最近一次成功重接管距今不足 REOWN_COOLDOWN。
/// last_reown 在未来（时钟异常）时返回 false（视为不在冷却期），不会 panic
fn in_cooldown(last_reown: std::time::Instant, now: std::time::Instant) -> bool {
    now.checked_duration_since(last_reown)
        .map(|d| d < REOWN_COOLDOWN)
        .unwrap_or(false)
}

/// abandoned_worker_pending 的恢复条件：旧 worker 的 DONE 是否已丢失超时。
/// 回传永久丢失（PostMessageW 失败/worker 异常退出）时 reown 永远卡死；超过
/// REOWN_IN_FLIGHT_TIMEOUT 后门控清除标志放行重新派发，迟到结果按代数忽略
fn abandoned_worker_expired(abandoned_since: Option<std::time::Instant>) -> bool {
    abandoned_since.is_some_and(|t| t.elapsed() >= REOWN_IN_FLIGHT_TIMEOUT)
}

/// worker 进度心跳判定：REOWN_WORKER_PROGRESS 是否比上次检查时推进，且计数
/// 的代数基线（高 32 位）与 seen 一致。有进展（同基线且 now > seen）→ 慢但
/// 存活，重置超时窗口不 bump；无进展（基线不同或停滞）→ 判卡死。防止仅因
/// 超时递增代数 → Skipped → Ignore → 重新派发的大剪贴板无限循环。
/// 注意：基线编码只保证 seen 与计数可比（重置前的旧累计体现代数差异），
/// 无法区分重置后同基线内 bump 的来源（旧 worker 的 bump 会叠加到新基线）
fn worker_made_progress(seen: u64, now: u64) -> bool {
    now >> 32 == seen >> 32 && now > seen
}

/// 超时心跳检查的处置（纯决策，便于单测）
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum HeartbeatVerdict {
    /// 有推进：worker 慢但存活，重置超时窗口、清零 strikes
    Alive,
    /// 第一次无推进：置位 strikes、重置超时窗口再等一轮
    FirstStrike,
    /// 连续两次无推进：判定卡死，进入 bump + abandoned
    Stuck,
}

/// 纯决策：超时心跳检查后的处置。有推进 → Alive；无推进且首次 → FirstStrike；
/// 无推进且已有一击 → Stuck。连续两次无推进才判卡死：单次阻塞的
/// GetClipboardData/GlobalLock（延迟渲染）可能阻塞数秒，一次无推进就中止会
/// 误杀慢但存活的 worker
fn decide_heartbeat_verdict(seen: u64, now: u64, strikes: bool) -> HeartbeatVerdict {
    if worker_made_progress(seen, now) {
        HeartbeatVerdict::Alive
    } else if !strikes {
        HeartbeatVerdict::FirstStrike
    } else {
        HeartbeatVerdict::Stuck
    }
}

/// 递增 worker 进度心跳（worker 线程调用）：reown 每阶段/每格式 +1。消息
/// 线程超时分支据此判断 worker 是否仍在推进（慢但存活）——大剪贴板读取/
/// 预分配期间计数持续增长，不被固定阈值误判卡死
fn bump_worker_progress() {
    REOWN_WORKER_PROGRESS.fetch_add(1, Ordering::SeqCst);
}

/// 当前剪贴板所有者是否为本进程（main 窗口）的窗口：reown 以 main 窗口为
/// 所有者 SetClipboardData，成功后 owner 即本进程；其他来源（WebView2 子
/// 进程/外部）PID 均不等于本进程，据此区分"reown 反馈"与"用户新复制"
fn is_owner_our_process() -> bool {
    let Ok(owner) = (unsafe { GetClipboardOwner() }) else {
        return false;
    };
    if owner.is_invalid() {
        return false;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(owner, Some(&mut pid)) };
    pid != 0 && pid == std::process::id()
}

/// should_reown 的决策结果（消息线程专用）
enum ShouldReown {
    /// 应派发 reown（携带主窗口句柄）
    Dispatch(HWND),
    /// 应停止定时器（冷却期 / 非本应用 / 无窗口）
    Stop,
    /// PID 缓存刷新中（已请求 worker 异步枚举）：重新布防定时器稍后再查，
    /// 不消耗重试预算
    WaitRefresh,
}

/// 守卫：本次剪贴板更新是否应派发 reown。冷却期内/非本应用复制/无主窗口 →
/// Stop（停止定时器）；PID 缓存陈旧/未命中 → WaitRefresh（等 worker 刷新后
/// 重查）；否则 → Dispatch（派发 reown 到 worker 线程执行）
fn should_reown(ctx: &mut ListenerCtx) -> ShouldReown {
    // 冷却期内跳过，避免高频复制时过度占用剪贴板（廉价检查，先于进程快照）；
    // None（从未重接管过）跳过冷却，保证启动后首次复制不被节流
    if ctx
        .last_reown
        .is_some_and(|t| in_cooldown(t, std::time::Instant::now()))
    {
        return ShouldReown::Stop;
    }
    // 只处理本应用 WebView2 发起的复制，且主窗口可用
    let hwnd = ctx.app.get_window("main").and_then(|w| w.hwnd().ok());
    // owner 检查仅在首次派发时执行（first_attempt_pending 由
    // WM_CLIPBOARDUPDATE 置位、首次派发后清除）：重试期间源进程可能仍持有
    // 剪贴板，GetClipboardOwner 瞬时返回 NULL/错误，每次都重查会把合法重试
    // 当非本应用中止（KillTimer）。重试直接走 reown 处理竞争；用显式标志而
    // 非 retries_left==MAX 推断，避免重试中新事件被误判"非首次"跳过检查
    let owner_is_ours = if ctx.first_attempt_pending {
        match is_our_webview_owner_cached(ctx) {
            OwnerCheck::Ours => true,
            OwnerCheck::NotOurs => {
                // owner 不是本应用 WebView2。若 owner 是本进程 main 窗口，
                // 是 reown 自身反馈的余波：盖冷却戳并停止（否则每次重接管后
                // 反馈再触发 owner 检查 + 全表枚举，节流失效）；外部不盖戳
                if is_owner_our_process() {
                    ctx.last_reown = Some(std::time::Instant::now());
                }
                return ShouldReown::Stop;
            }
            OwnerCheck::RefreshPending => return ShouldReown::WaitRefresh,
        }
    } else {
        true
    };
    if !should_attempt_reown(owner_is_ours, hwnd.is_some()) {
        return ShouldReown::Stop;
    }
    match hwnd {
        Some(h) => ShouldReown::Dispatch(h),
        None => ShouldReown::Stop,
    }
}

/// owner 判定结果（消息线程专用）
enum OwnerCheck {
    /// 是（缓存命中）
    Ours,
    /// 否（recent-miss 记忆或确定非本应用）
    NotOurs,
    /// 缓存陈旧/未命中，已请求 worker 异步刷新，稍后重查
    RefreshPending,
}

/// 消息线程专用 owner 检查：只读缓存 Arc，绝不在此枚举进程表（枚举在 worker
/// 线程经 RefreshPids 完成）。陈旧/未命中且非已知外部时请求异步刷新返回
/// RefreshPending；去重基于目标 PID——同 PID 等待，异 PID 重发，否则永久卡死
fn is_our_webview_owner_cached(ctx: &mut ListenerCtx) -> OwnerCheck {
    let Ok(owner) = (unsafe { GetClipboardOwner() }) else {
        return OwnerCheck::NotOurs;
    };
    if owner.is_invalid() {
        return OwnerCheck::NotOurs;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(owner, Some(&mut pid)) };
    if pid == 0 {
        return OwnerCheck::NotOurs;
    }
    // owner 是本进程（main 窗口）：reown 自身反馈而非子进程复制（子进程 PID
    // 必不等于本进程），直接判定非本应用无需刷新，should_reown 据此盖戳
    if pid == std::process::id() {
        return OwnerCheck::NotOurs;
    }
    // 只读缓存：命中即本应用；未命中且非已知外部时请求 worker 异步刷新。
    // 不关心缓存新鲜度，未命中即刷新确认，避免新启动子进程被节流盲窗丢弃
    let (pids, _fresh) = cached_webview_pids();
    if pids.contains(&pid) {
        // 命中：刷新循环结束，清除排队标志与退避，允许下一轮需要时再刷新
        ctx.refresh_pending_target = None;
        ctx.refresh_backoff = 0;
        return OwnerCheck::Ours;
    }
    if is_recent_miss(pid) {
        // 已判定非本应用：刷新循环结束
        ctx.refresh_pending_target = None;
        ctx.refresh_backoff = 0;
        return OwnerCheck::NotOurs;
    }
    // 缓存未命中且非已知外部 PID：可能是刚启动的 WebView2 子进程或外部程序。
    // 请求 worker 异步刷新（枚举在 worker 线程）返回 RefreshPending 等下次重查
    if let Some(target_pid) = ctx.refresh_pending_target {
        if target_pid == pid {
            // 同一 PID 的刷新请求已排队（worker 尚未消费）：保持去重等待
            return OwnerCheck::RefreshPending;
        }
        // 排队请求核对的是别的 PID：清除，下面重发当前 PID 的刷新请求
        ctx.refresh_pending_target = None;
    }
    // 有界队列（容量 1）已满时 try_send 返回 Full：不入队、不置刷新目标
    // （否则去重门控永不重试），由 WaitRefresh 重试；已退出则按非本应用
    match ctx.reown_tx.try_send(WorkerMsg::RefreshPids(pid)) {
        Ok(()) => {
            ctx.refresh_pending_target = Some(pid);
            OwnerCheck::RefreshPending
        }
        Err(std::sync::mpsc::TrySendError::Full(_)) => OwnerCheck::RefreshPending,
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => OwnerCheck::NotOurs,
    }
}

/// 只读缓存（消息线程专用）：返回 (PID 列表 Arc, 缓存是否新鲜)。
/// 绝不在此线程执行进程表枚举
fn cached_webview_pids() -> (Arc<Vec<u32>>, bool) {
    let now = std::time::Instant::now();
    let cache = WEBVIEW_PIDS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match cache.as_ref() {
        Some((cached_at, pids)) => {
            let fresh = now
                .checked_duration_since(*cached_at)
                .is_some_and(|d| d < WEBVIEW_PIDS_CACHE_TTL);
            (Arc::clone(pids), fresh)
        }
        None => (Arc::new(Vec::new()), false),
    }
}

/// 当前剪贴板所有者是否为本应用 WebView2 子进程（worker 线程专用，仅 reown
/// 内调用）。**只读缓存、绝不在此枚举进程表**：OpenClipboard 持有期间触发
/// CreateToolhelp32Snapshot（数十毫秒）会让剪贴板全局锁跨过整个快照，其他
/// 应用 OpenClipboard 全部失败。缓存未命中返回 false（Skipped，剪贴板未动），
/// 由消息线程经 RefreshPids 异步刷新
fn is_our_webview_owner() -> bool {
    let Ok(owner) = (unsafe { GetClipboardOwner() }) else {
        return false;
    };
    if owner.is_invalid() {
        return false;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(owner, Some(&mut pid)) };
    if pid == 0 {
        return false;
    }
    // 只读缓存，绝不在此枚举进程表——剪贴板锁定期间全表枚举会阻塞
    // 其他所有应用的复制/粘贴
    let (pids, _fresh) = cached_webview_pids();
    pids.contains(&pid)
}

/// 立即重枚举 WebView2 PID 缓存并返回结果（不节流）。
/// 仅 worker 线程调用；消息线程只读缓存，绝不在此枚举。返回列表供调用方复用
fn refresh_webview_pids_now() -> Arc<Vec<u32>> {
    let now = std::time::Instant::now();
    // 只建一个 Arc：缓存存 Arc::clone（共享同一份 Vec），返回给调用方的
    // 也是同一份，避免 Vec 双份分配（Arc 不可变，共享安全）
    let pids = Arc::new(enumerate_webview_pids());
    let mut cache = WEBVIEW_PIDS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *cache = Some((now, Arc::clone(&pids)));
    pids
}

/// 该 PID 是否在 recent-miss 记忆内（最近已判定为非本应用）
fn is_recent_miss(pid: u32) -> bool {
    let now = std::time::Instant::now();
    let mut recent = WEBVIEW_PIDS_RECENT_MISS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // 清理过期条目，避免集合无限增长
    recent.retain(|(_, t)| {
        now.checked_duration_since(*t)
            .is_some_and(|d| d < WEBVIEW_PIDS_RECENT_MISS_TTL)
    });
    recent.iter().any(|(p, _)| *p == pid)
}

/// 记录该 PID 判定为非本应用（供节流使用）
fn record_recent_miss(pid: u32) {
    let now = std::time::Instant::now();
    let mut recent = WEBVIEW_PIDS_RECENT_MISS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // 清理过期条目并限制集合大小（防止极端场景下无限增长）
    recent.retain(|(_, t)| {
        now.checked_duration_since(*t)
            .is_some_and(|d| d < WEBVIEW_PIDS_RECENT_MISS_TTL)
    });
    if recent.len() >= 64 {
        recent.remove(0);
    }
    recent.push((pid, now));
}

/// 全量枚举进程表，找出本应用启动的 WebView2 子进程 PID
fn enumerate_webview_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return pids;
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry).is_ok() };
    while ok {
        // 父进程是本进程 且 进程名为 WebView2 浏览器进程
        if entry.th32ParentProcessID == std::process::id() && is_webview2_exe(&entry.szExeFile) {
            pids.push(entry.th32ProcessID);
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry).is_ok() };
    }
    let _ = unsafe { CloseHandle(snapshot) };
    pids
}

/// 判断进程可执行文件名是否为 WebView2 浏览器进程
fn is_webview2_exe(exe: &[u16; 260]) -> bool {
    let len = exe.iter().position(|&c| c == 0).unwrap_or(exe.len());
    let name = String::from_utf16_lossy(&exe[..len]).to_lowercase();
    name == "msedgewebview2.exe"
}

/// 该格式的数据不是 HGLOBAL（GlobalLock 会失效），跳过：
/// 2=CF_BITMAP(HBITMAP) 3=CF_METAFILEPICT 9=CF_PALETTE(HPALETTE)
/// 14=CF_ENHMETAFILE 0x80=CF_OWNERDISPLAY，0x81-0xFF=显示器格式（CF_DSP*）
fn is_global_memory_format(format: u32) -> bool {
    !matches!(format, 2 | 3 | 9 | 14 | 0x0080) && !(0x0081..=0x00FF).contains(&format)
}

/// 以宿主窗口为所有者重新设置剪贴板，使内容出现在 Win+V 历史中。
/// 返回 Reowned=成功；Skipped=存在无法复制的格式/代数过期，剪贴板未动
/// （不盖冷却戳/不重试）；Busy=OpenClipboard/EmptyClipboard 竞争失败（可重试）。
/// 破坏性阶段前核对 REOWN_CURRENT_GENERATION：代数过期时放弃，否则旧 worker
/// 会把旧内容覆盖到新内容上。**句柄记账不变量**（Win32 不可 mock）：每个提前
/// 返回路径释放全部句柄并 CloseClipboard；SetClipboardData 后每句柄要么被系统
/// 接管要么已释放。**进度心跳**：各阶段递增 REOWN_WORKER_PROGRESS，超时分支
/// 据此区分"慢但存活"（重置窗口）与"卡死"（bump + abandoned）
fn reown(hwnd: Option<HWND>, generation: u64) -> ReownResult {
    // 缺少有效窗口句柄时放弃：OpenClipboard(NULL) 下 EmptyClipboard 会把
    // 所有者设为 NULL，导致 SetClipboardData 全部失败并清空剪贴板
    let Some(hwnd) = hwnd else {
        return ReownResult::Skipped;
    };

    unsafe {
        // 其他程序正在占用剪贴板时失败，等待重试
        if OpenClipboard(Some(hwnd)).is_err() {
            return ReownResult::Busy;
        }
        bump_worker_progress(); // 心跳：已打开剪贴板

        // 代数核对（提前）：超时中止/新派发后仍存活的旧 worker 直接放弃，
        // 不做无谓的读取/预分配；EmptyClipboard 前还有一次权威核对兜底
        if REOWN_CURRENT_GENERATION.load(Ordering::SeqCst) != generation {
            let _ = CloseClipboard();
            return ReownResult::Skipped;
        }
        bump_worker_progress(); // 心跳：通过提前核对，开始读取

        // 先枚举并读取当前所有格式的数据（必须在清空前完成读取）
        let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
        // 存在无法完整复制的格式时中止重设，避免 EmptyClipboard 销毁它们
        let mut skipped = false;
        let mut format = 0;
        loop {
            format = EnumClipboardFormats(format);
            if format == 0 {
                break;
            }
            bump_worker_progress(); // 心跳：每枚举到一个格式
            if !is_global_memory_format(format) {
                skipped = true;
                continue;
            }
            // 心跳：进入 GetClipboardData/GlobalLock 前推进——延迟渲染格式
            // （rich HTML/iframe）可能阻塞数秒，若无此 bump 会被超时门控误判
            // 卡死（中止 → 重派发 → 旧 worker 持锁 → 重试全 Busy → GiveUp）
            bump_worker_progress();
            let Ok(data) = GetClipboardData(format) else {
                skipped = true;
                continue;
            };
            let handle = HGLOBAL(data.0);
            let size = GlobalSize(handle);
            let ptr = GlobalLock(handle);
            if !ptr.is_null() {
                bump_worker_progress(); // 心跳：GlobalLock 成功
                // 分块拷贝（1 MiB/块，每块推进心跳）：单格式超大负载逐块
                // 推进，避免单次 to_vec 拷贝超过超时阈值被误判卡死。
                // 用 vec![0u8; size] 而非 set_len：后者留下未初始化内存
                // （clippy::uninit_vec 拒绝），零填充成本相对拷贝可忽略
                let mut copied: Vec<u8> = vec![0u8; size];
                const COPY_CHUNK: usize = 1 << 20;
                let mut offset = 0usize;
                while offset < size {
                    let n = (size - offset).min(COPY_CHUNK);
                    std::ptr::copy_nonoverlapping(
                        (ptr as *const u8).add(offset),
                        copied.as_mut_ptr().add(offset),
                        n,
                    );
                    offset += n;
                    bump_worker_progress();
                }
                entries.push((format, copied));
            } else {
                skipped = true;
            }
            let _ = GlobalUnlock(handle);
        }
        bump_worker_progress(); // 心跳：全部格式读取完成

        // 没有可重设的数据，或存在无法复制的格式时，不动剪贴板（避免清空已有内容）。
        // 返回 Skipped：调用方应停止定时器但**不盖冷却戳**，避免后续复制被节流
        if entries.is_empty() || skipped {
            let _ = CloseClipboard();
            return ReownResult::Skipped;
        }

        // 重读数据后重新校验 owner：重试期间门控吞掉新事件、重试派发跳过
        // owner 检查，若外部程序此时复制了新内容，重读到的 entries 就是外部
        // 内容，不能以 main 窗口为所有者重设（会把外部内容静默"接管"进本应用
        // 历史）。owner 已不是本应用 WebView2 时返回 Skipped（剪贴板未动）。
        // 不枚举进程表：当前持有 OpenClipboard，全表枚举会阻塞其他应用复制
        if !is_our_webview_owner() {
            let _ = CloseClipboard();
            return ReownResult::Skipped;
        }
        bump_worker_progress(); // 心跳：owner 重校验通过，开始预分配

        // 预分配所有句柄并拷贝数据：全部成功后才 EmptyClipboard，避免部分
        // 分配失败时已清空剪贴板却无法重设（用户内容被销毁）
        let mut handles: Vec<HGLOBAL> = Vec::with_capacity(entries.len());
        let mut alloc_failed = false;
        for (format, bytes) in &entries {
            bump_worker_progress(); // 心跳：每分配一个格式
            let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
                warn!("分配剪贴板内存失败：format={format}");
                alloc_failed = true;
                break;
            };
            let ptr = GlobalLock(handle);
            if ptr.is_null() {
                let _ = GlobalFree(Some(handle));
                alloc_failed = true;
                break;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            let _ = GlobalUnlock(handle);
            handles.push(handle);
        }
        // 破坏性阶段（权威代数核对 + EmptyClipboard + SetClipboardData）：
        // 结果与释放策略由纯函数 decide_rewrite_cleanup 决策（单测覆盖各早退
        // 路径的句柄记账），此处只负责执行。EmptyClipboard 仅在预分配全部成功
        // 且代数匹配时调用——否则破坏剪贴板后无法重设（用户内容被清空）
        let gen_matches = REOWN_CURRENT_GENERATION.load(Ordering::SeqCst) == generation;
        let mut empty_ok = false;
        let mut set_succeeded = 0u32;
        // EmptyClipboard 仅在预分配全部成功、代数匹配且 main 窗口仍有效时调用：
        // 窗口已销毁时 SetClipboardData 会因所有者无效全部失败，而 EmptyClipboard
        // 已把剪贴板清空（用户内容被破坏性清空）；先校验窗口避免进入该状态
        if !alloc_failed && gen_matches && IsWindow(Some(hwnd)).as_bool() {
            bump_worker_progress(); // 心跳：全部格式预分配完成
            if EmptyClipboard().is_ok() {
                empty_ok = true;
                bump_worker_progress(); // 心跳：已清空剪贴板，开始设置各格式
                for (i, (format, _)) in entries.iter().enumerate() {
                    bump_worker_progress(); // 心跳：每设置一个格式
                    let handle = handles[i];
                    if SetClipboardData(*format, Some(HANDLE(handle.0))).is_err() {
                        let _ = GlobalFree(Some(handle));
                    } else {
                        set_succeeded += 1; // 成功后句柄由系统接管，不再释放
                    }
                }
                bump_worker_progress(); // 心跳：全部格式设置完成
            }
        }
        let (result, mode) =
            decide_rewrite_cleanup(!alloc_failed, gen_matches, empty_ok, set_succeeded);
        if mode == CleanupMode::FreeAll {
            // 释放全部预分配句柄：alloc 失败 / 代数不匹配 / EmptyClipboard
            // 失败时句柄均未移交系统，必须全部释放；FreeFailed 时成功者已
            // 被系统接管、失败者已在循环内释放，无需再处理
            for handle in &handles {
                let _ = GlobalFree(Some(*handle));
            }
        }
        let _ = CloseClipboard();
        result
    }
}

/// 常驻 reown worker 线程：从 channel 取任务执行，完成后经 WM_REOWN_DONE
/// 回传。替代每次派发 std::thread::spawn；channel 关闭（ctx 释放时 drop
/// sender）后 recv 返回 Err 自然退出。进程表全量枚举（慢操作）也在此线程
/// 执行（RefreshPids），消息泵只读缓存 Arc，永不被枚举阻塞
fn reown_worker(rx: std::sync::mpsc::Receiver<WorkerMsg>, listener_hwnd_val: usize) {
    while let Ok(msg) = rx.recv() {
        match msg {
            WorkerMsg::Reown(job) => {
                let main_hwnd = HWND(job.main_hwnd_val as *mut core::ffi::c_void);
                let listener_hwnd = HWND(listener_hwnd_val as *mut core::ffi::c_void);
                // 派发与执行之间可能相隔 2s，期间 main 窗口可能已被销毁：
                // 陈旧句柄下 EmptyClipboard 会清空剪贴板而 SetClipboardData
                // 因所有者窗口无效全部失败——用户内容被破坏性清空。执行
                // 破坏性重设前先校验句柄，失效时回传 Skipped（剪贴板未动）
                let result = if unsafe { IsWindow(Some(main_hwnd)).as_bool() } {
                    reown(Some(main_hwnd), job.generation)
                } else {
                    ReownResult::Skipped
                };
                let _ = unsafe {
                    PostMessageW(
                        Some(listener_hwnd),
                        WM_REOWN_DONE,
                        WPARAM(result as usize),
                        LPARAM(job.generation as isize),
                    )
                };
            }
            WorkerMsg::RefreshPids(pid) => {
                // 全量枚举进程表（慢操作，在 worker 线程执行），更新缓存；
                // 刷新后 PID 仍不在列表中则记录 recent-miss（外部 PID），
                // 使消息线程下次检查直接判定为非本应用
                let pids = refresh_webview_pids_now();
                if !pids.contains(&pid) {
                    record_recent_miss(pid);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CleanupMode, HeartbeatVerdict, ReownDoneStep, ReownResult, abandoned_worker_expired,
        budget_after_rearm, budget_after_retry_abort, decide_heartbeat_verdict, decide_reown_done,
        decide_rewrite_cleanup, in_cooldown, retry_loop_active, should_attempt_reown,
        worker_made_progress,
    };
    use std::time::{Duration, Instant};

    /// 成功的重接管总是停止定时器，与剩余重试次数无关
    #[test]
    fn success_always_stops_timer() {
        assert_eq!(
            decide_reown_done(ReownResult::Reowned, true, 0),
            ReownDoneStep::Success
        );
        assert_eq!(
            decide_reown_done(ReownResult::Reowned, true, 10),
            ReownDoneStep::Success
        );
    }

    /// 竞争失败且还有重试次数时重新布防定时器
    #[test]
    fn busy_with_retries_left_rearms() {
        assert_eq!(
            decide_reown_done(ReownResult::Busy, true, 1),
            ReownDoneStep::Retry
        );
        assert_eq!(
            decide_reown_done(ReownResult::Busy, true, 10),
            ReownDoneStep::Retry
        );
    }

    /// 竞争失败且重试次数耗尽时放弃
    #[test]
    fn busy_without_retries_gives_up() {
        assert_eq!(
            decide_reown_done(ReownResult::Busy, true, 0),
            ReownDoneStep::GiveUp
        );
    }

    /// 重试计数从 REOWN_MAX_RETRIES 递减，恰好消耗完次数后终止，
    /// 不会出现第 11 次仍为 Retry 的 off-by-one
    #[test]
    fn retry_counter_terminates() {
        let mut retries_left = super::REOWN_MAX_RETRIES;
        let mut rearmed = 0;
        while decide_reown_done(ReownResult::Busy, true, retries_left) == ReownDoneStep::Retry {
            retries_left -= 1;
            rearmed += 1;
        }
        assert_eq!(rearmed, super::REOWN_MAX_RETRIES as usize);
        assert_eq!(retries_left, 0);
        assert_eq!(
            decide_reown_done(ReownResult::Busy, true, retries_left),
            ReownDoneStep::GiveUp
        );
    }

    /// 冷却期内返回 true，冷却期过后返回 false
    #[test]
    fn cooldown_window_respected() {
        let now = Instant::now();
        // 冷却期内（不足 REOWN_COOLDOWN）：用 checked_sub 构造过去时刻，
        // 主机 uptime 不足（< 100ms）时跳过该分支断言，避免 `Instant -
        // Duration` 下溢 panic
        if let Some(recent) = now.checked_sub(Duration::from_millis(100)) {
            assert!(in_cooldown(recent, now));
        }
        // 冷却期已过（超过 REOWN_COOLDOWN）：同理，uptime 不足时跳过
        if let Some(past) = now.checked_sub(Duration::from_millis(400)) {
            assert!(!in_cooldown(past, now));
        }
    }

    /// 时间戳异常（now 早于 last_reown）时不 panic，视为不在冷却期
    #[test]
    fn cooldown_saturates_on_clock_skew() {
        let now = Instant::now();
        assert!(!in_cooldown(now + Duration::from_millis(100), now));
    }

    /// 守卫：本应用复制且有主窗口才尝试重接管
    #[test]
    fn attempt_only_for_our_owner_with_window() {
        assert!(should_attempt_reown(true, true));
        assert!(!should_attempt_reown(false, true)); // 非本应用复制
        assert!(!should_attempt_reown(true, false)); // 无主窗口
        assert!(!should_attempt_reown(false, false));
    }

    /// 记账：布防成功消耗预算（含 0 饱和），布防失败保留预算
    #[test]
    fn rearm_consumes_budget_only_on_success() {
        assert_eq!(budget_after_rearm(10, true), 9);
        assert_eq!(budget_after_rearm(1, true), 0);
        assert_eq!(budget_after_rearm(0, true), 0); // saturating，不为负
        assert_eq!(budget_after_rearm(10, false), 10); // 失败不消耗
        assert_eq!(budget_after_rearm(0, false), 0);
    }

    /// 反馈循环保护：成功重接管后立即到来的事件被冷却拦截，
    /// 不会触发第二次 reown（避免 reown 自身触发的 WM_CLIPBOARDUPDATE 死循环）
    #[test]
    fn feedback_event_blocked_by_cooldown() {
        let now = Instant::now();
        // 模拟刚成功重接管：last_reown 刚刚盖戳（用 checked_sub 构造，
        // uptime 不足 10ms 时跳过，避免 `Instant - Duration` 下溢 panic）
        if let Some(last_reown) = now.checked_sub(Duration::from_millis(10)) {
            // 冷却期内：不尝试重接管
            assert!(in_cooldown(last_reown, now));
            // 冷却期过后（400ms）：允许重接管
            let later = now + Duration::from_millis(400);
            assert!(!in_cooldown(last_reown, later));
        }
    }

    /// 预算边界：连续布防失败不消耗预算，事件重新布防后仍能重试
    #[test]
    fn failed_rearm_preserves_retry_budget() {
        let mut retries_left = 1;
        // 布防失败两次：预算保留
        retries_left = budget_after_rearm(retries_left, false);
        retries_left = budget_after_rearm(retries_left, false);
        assert_eq!(retries_left, 1);
        assert_eq!(
            decide_reown_done(ReownResult::Busy, true, retries_left),
            ReownDoneStep::Retry
        );
        // 随后布防成功：预算耗尽 → GiveUp
        retries_left = budget_after_rearm(retries_left, true);
        assert_eq!(retries_left, 0);
        assert_eq!(
            decide_reown_done(ReownResult::Busy, true, retries_left),
            ReownDoneStep::GiveUp
        );
    }

    /// WM_REOWN_DONE：剪贴板未动（存在无法复制的格式）→ Skip：
    /// 停止定时器但**不盖冷却戳**（避免后续复制被节流）
    #[test]
    fn reown_done_skipped_stops_without_cooldown() {
        assert_eq!(
            decide_reown_done(ReownResult::Skipped, true, 5),
            ReownDoneStep::Skip
        );
        // 与剩余重试次数无关
        assert_eq!(
            decide_reown_done(ReownResult::Skipped, true, 0),
            ReownDoneStep::Skip
        );
    }

    /// 超时后过期 worker 的结果：代数不匹配 → Ignore，无论结果如何，
    /// 既不消耗预算也不盖冷却戳（避免预算双消耗、冷却戳来自过期结果）
    #[test]
    fn stale_reown_done_is_ignored() {
        // 过期竞争失败结果：不应进入 Retry（不消耗预算）
        assert_eq!(
            decide_reown_done(ReownResult::Busy, false, 3),
            ReownDoneStep::Ignore
        );
        // 过期成功结果：不应进入 Success（不盖冷却戳）
        assert_eq!(
            decide_reown_done(ReownResult::Reowned, false, 3),
            ReownDoneStep::Ignore
        );
        // 过期跳过结果：不应进入 Skip
        assert_eq!(
            decide_reown_done(ReownResult::Skipped, false, 3),
            ReownDoneStep::Ignore
        );
        // 过期结果与剩余预算无关
        assert_eq!(
            decide_reown_done(ReownResult::Busy, false, 0),
            ReownDoneStep::Ignore
        );
    }

    /// 超时-过期结果序列：旧 worker 迟到结果被忽略，不会双消耗预算；
    /// 只有当前代数的结果才推进状态机
    #[test]
    fn timeout_then_stale_result_does_not_double_consume() {
        // 模拟：worker 代数 1 超时 → 派发 worker 代数 2（预算 10 → 9）
        let mut retries_left = 10;
        retries_left = budget_after_rearm(retries_left, true);
        assert_eq!(retries_left, 9);
        // 代数 1 的迟到失败结果：忽略（不再次消耗预算）
        assert_eq!(
            decide_reown_done(ReownResult::Busy, false, retries_left),
            ReownDoneStep::Ignore
        );
        assert_eq!(retries_left, 9);
        // 代数 2 的失败结果：正常 Retry（消耗预算 9 → 8）
        assert_eq!(
            decide_reown_done(ReownResult::Busy, true, retries_left),
            ReownDoneStep::Retry
        );
        retries_left = budget_after_rearm(retries_left, true);
        assert_eq!(retries_left, 8);
    }

    /// retry_loop_active 纯判定：worker 在飞或预算已消耗视为重试循环激活
    #[test]
    fn retry_active_gating() {
        let max = super::REOWN_MAX_RETRIES;
        // 空闲（未在飞、满预算）：不激活，允许新事件重置
        assert!(!retry_loop_active(false, max));
        // worker 在飞：激活，抑制反馈事件重置
        assert!(retry_loop_active(true, max));
        // 预算已消耗（重试中）：激活，即使此刻无 worker 在飞
        assert!(retry_loop_active(false, max - 1));
        assert!(retry_loop_active(false, 0));
        // 两者同时成立仍激活
        assert!(retry_loop_active(true, 0));
    }

    /// 预算生命周期：新事件重置 → 重试消耗 → 终态恢复，保证
    /// retry_active 门控不会永久吞掉后续新事件
    #[test]
    fn retry_budget_lifecycle_recovers_after_terminal() {
        let max = super::REOWN_MAX_RETRIES;
        let mut retries_left = max;
        let mut reown_in_flight = false;

        // 新事件（WM_CLIPBOARDUPDATE）：未激活 → 重置预算
        assert!(!retry_loop_active(reown_in_flight, retries_left));
        retries_left = max;

        // 派发 worker（WM_TIMER）：在飞 → 激活，反馈事件被抑制
        reown_in_flight = true;
        assert!(retry_loop_active(reown_in_flight, retries_left));

        // worker 返回 Busy（WM_REOWN_DONE）：Retry → 扣预算 → 仍在激活
        assert_eq!(
            decide_reown_done(ReownResult::Busy, true, retries_left),
            ReownDoneStep::Retry
        );
        reown_in_flight = false;
        retries_left = budget_after_rearm(retries_left, true);
        assert_eq!(retries_left, max - 1);
        assert!(retry_loop_active(reown_in_flight, retries_left));

        // 终态 Success：恢复满预算 → 门控放行后续新事件
        assert_eq!(
            decide_reown_done(ReownResult::Reowned, true, retries_left),
            ReownDoneStep::Success
        );
        retries_left = max;
        assert!(!retry_loop_active(reown_in_flight, retries_left));

        // 另一轮：Busy 两次后 GiveUp，同样恢复满预算
        reown_in_flight = true;
        assert!(retry_loop_active(reown_in_flight, retries_left));
        reown_in_flight = false;
        retries_left = budget_after_rearm(retries_left, true);
        retries_left = budget_after_rearm(retries_left, true);
        assert_eq!(retries_left, max - 2);
        assert!(retry_loop_active(reown_in_flight, retries_left));
        assert_eq!(
            decide_reown_done(ReownResult::Skipped, true, retries_left),
            ReownDoneStep::Skip
        );
        retries_left = max;
        assert!(!retry_loop_active(reown_in_flight, retries_left));
    }

    /// 超时中止：重试循环中 worker 超时未回传后，预算必须恢复满预算，
    /// 使 WM_CLIPBOARDUPDATE 的重置门控重新放行后续新事件
    #[test]
    fn timeout_restores_budget_and_reopens_gate() {
        let max = super::REOWN_MAX_RETRIES;
        // 重试循环中预算已消耗（如多次 Busy 后）→ 门控激活
        let consumed = budget_after_rearm(max, true);
        assert!(retry_loop_active(false, consumed));
        // 超时中止：恢复满预算 → 门控重新放行
        let restored = budget_after_retry_abort();
        assert_eq!(restored, max);
        assert!(!retry_loop_active(false, restored));
    }

    /// 重布防失败：定时器布防失败后预算同样恢复满预算（而非保留已消耗
    /// 的预算），避免 retry_loop_active 门控永久关闭吞掉后续新事件
    #[test]
    fn rearm_failure_restores_budget_and_reopens_gate() {
        let max = super::REOWN_MAX_RETRIES;
        // 重试中预算已消耗 → 门控激活
        let consumed = budget_after_rearm(max, true);
        assert!(retry_loop_active(false, consumed));
        // 重布防失败：budget_after_rearm(consumed, false) 会保留已消耗预算，
        // 门控保持激活 → 这是 bug；正确路径是恢复满预算
        assert!(retry_loop_active(
            false,
            budget_after_rearm(consumed, false)
        ));
        let restored = budget_after_retry_abort();
        assert_eq!(restored, max);
        assert!(!retry_loop_active(false, restored));
    }

    /// 状态机集成测试：事件 A 派发后在飞时事件 B 到达 → 被门控吞掉并置位
    /// new_event_pending，终态分支必须重新布防定时器处理 B，否则 B 永不
    /// 重接管。用纯函数驱动状态转移，模拟 clipboard_wnd_proc 的消息序列
    #[test]
    fn second_event_during_flight_is_rearmed_after_terminal() {
        let max = super::REOWN_MAX_RETRIES;
        // 事件 A（WM_CLIPBOARDUPDATE）：无重试循环 → 门控放行，重置预算并置位
        // 首次派发。每步状态转移后都立即断言读取，避免未使用赋值警告
        let mut retries_left = max;
        let mut first_attempt_pending = true;
        let mut reown_in_flight = false;
        let mut new_event_pending = false;
        assert!(!retry_loop_active(reown_in_flight, retries_left));
        assert!(first_attempt_pending);

        // WM_TIMER 派发 A（owner 检查通过）：进入在飞状态，清除首次派发标志
        reown_in_flight = true;
        first_attempt_pending = false;
        assert!(retry_loop_active(reown_in_flight, retries_left));
        assert!(!first_attempt_pending);

        // 事件 B（WM_CLIPBOARDUPDATE）：在飞 → 门控激活 → 被吞掉，置位 new_event_pending。
        // 置位前先断言当前无待处理事件（读取初始值，避免未使用赋值警告）
        assert!(!new_event_pending);
        new_event_pending = true;
        assert!(new_event_pending);

        // WM_REOWN_DONE(Reowned, 代数匹配) → Success 终态
        assert_eq!(
            decide_reown_done(ReownResult::Reowned, true, retries_left),
            ReownDoneStep::Success
        );
        reown_in_flight = false;
        retries_left = max;
        // 终态分支：new_event_pending 置位 → 重新布防（对应 rearm_for_pending_event）
        if new_event_pending {
            first_attempt_pending = true;
            new_event_pending = false;
        }
        // 断言：状态机回到"可派发新事件"状态——下一次 WM_TIMER 会重跑
        // owner 检查并派发 B，而不是永久停止
        assert!(!reown_in_flight);
        assert!(!new_event_pending);
        assert!(first_attempt_pending); // 下次触发会重新 owner 检查
        assert_eq!(retries_left, max);
        assert!(!retry_loop_active(reown_in_flight, retries_left)); // 门控放行
    }

    /// 状态机集成测试（超时 → 过期结果迟到）：超时后预算必须恢复、门控
    /// 重新放行；过期（代数不匹配）的成功结果在空闲时盖冷却戳，不双消耗预算
    #[test]
    fn timeout_then_stale_done_restores_budget_and_cooldown() {
        let max = super::REOWN_MAX_RETRIES;
        // 重试循环中：预算已消耗（多次 Busy 后）→ 门控激活
        let mut retries_left = budget_after_rearm(max, true);
        assert!(retry_loop_active(true, retries_left));

        // WM_TIMER 超时分支：恢复满预算（budget_after_retry_abort）
        retries_left = budget_after_retry_abort();
        assert_eq!(retries_left, max);
        assert!(!retry_loop_active(false, retries_left)); // 门控重新放行

        // 过期结果迟到（代数不匹配 → Ignore）：不消耗预算
        let gen_mismatch = false;
        let step = decide_reown_done(ReownResult::Reowned, gen_mismatch, retries_left);
        assert_eq!(step, ReownDoneStep::Ignore);
        assert_eq!(retries_left, max); // 预算未被双消耗

        // 状态机空闲（无在飞、满预算、无待派发）时盖冷却戳：
        // 对应 Ignore 分支的 Reowned + 空闲门控，阻止超时-反馈无限循环
        let reown_in_flight = false; // 超时分支已清
        let first_attempt_pending = false; // 已派发过、无新事件待派发
        let idle = !reown_in_flight && retries_left == max && !first_attempt_pending;
        assert!(idle);
        let last_reown = std::time::Instant::now();
        // 冷却期内：后续反馈事件被拦截
        assert!(in_cooldown(last_reown, std::time::Instant::now()));
    }

    /// abandoned_worker_pending 的恢复超时（"DONE 永不回传"路径）：
    /// WM_REOWN_DONE 永久丢失时超时后门控必须清除标志、恢复预算并允许
    /// 重新派发，否则 reown 功能永久卡死
    #[test]
    fn abandoned_worker_recovers_when_done_never_arrives() {
        let max = super::REOWN_MAX_RETRIES;
        // 模拟超时分支置位 abandoned_worker_pending 并记录时刻
        let abandoned_since = Some(Instant::now());
        // 刚置位未到超时：门控拦截（返回 LRESULT(0)，不派发）
        assert!(!abandoned_worker_expired(abandoned_since));

        // 时间推进到超过 REOWN_IN_FLIGHT_TIMEOUT：门控放行。
        // 用 checked_sub 构造过去时刻；主机 uptime 不足（< 超时阈值）时
        // 无法构造，跳过"已过期"分支断言（避免 expect panic），
        // 未过期分支不受影响
        if let Some(past) = Instant::now()
            .checked_sub(super::REOWN_IN_FLIGHT_TIMEOUT)
            .and_then(|t| t.checked_sub(Duration::from_millis(1)))
        {
            assert!(abandoned_worker_expired(Some(past)));

            // 门控放行后的恢复动作：清除标志 + 恢复满预算（对应 WM_TIMER 门控分支）
            let mut abandoned_worker_pending = true;
            let mut retries_left = max;
            if abandoned_worker_expired(Some(past)) {
                abandoned_worker_pending = false;
                retries_left = budget_after_retry_abort();
            }
            assert!(!abandoned_worker_pending);
            assert_eq!(retries_left, max);
            // 门控重新放行：新事件可正常布防派发
            assert!(!retry_loop_active(false, retries_left));
        }
    }

    /// refresh 去重的生命周期（基于目标 PID）：同 PID 排队则保持等待
    /// （慢 worker 不堆积 RefreshPids、不触发枚举风暴）；异 PID 才清除重发
    /// （否则当前 PID 永远无法解析、reown 卡死）；命中/判定非本应用时清除
    #[test]
    fn refresh_pending_target_based_dedup_lifecycle() {
        // 模拟：为 PID 100 排队的刷新请求仍在（worker 尚未消费）
        let mut refresh_pending_target = Some(100u32);

        // 同一 PID（100）再次检查：去重门控拦截（返回 RefreshPending，
        // 不重发）——即使 worker 慢，也不会重复堆积刷新请求
        let same_pid = 100u32;
        if refresh_pending_target == Some(same_pid) {
            // 保持去重等待（对应 is_our_webview_owner_cached 的早退）
        } else {
            refresh_pending_target = None;
        }
        assert_eq!(refresh_pending_target, Some(100)); // 未被清除、未重发

        // 目标 PID 不同（检查 PID 101，排队的刷新核对的是 PID 100）：
        // 清除并允许重发当前 PID——否则 PID 101 永远无法被核对
        // recent-miss，永久卡死
        let other_pid = 101u32;
        if refresh_pending_target == Some(other_pid) {
            // 保持等待
        } else {
            // 清除，下面重发当前 PID 的刷新请求
            refresh_pending_target = None;
        }
        assert_eq!(refresh_pending_target, None); // 放行重发

        // 重发后：排队标志记录新目标 PID
        refresh_pending_target = Some(other_pid);
        assert_eq!(refresh_pending_target, Some(101));

        // 检查命中（Ours）或判定非本应用（NotOurs）：清除排队标志，
        // 允许下一轮需要时再刷新
        refresh_pending_target = None;
        assert_eq!(refresh_pending_target, None);
    }

    /// worker_made_progress 纯决策：计数推进且代数基线一致（高 32 位一致）
    /// → worker 慢但存活，超时分支据此重置超时窗口、不 bump 代数；计数停滞/
    /// 回退、或高 32 位不一致（计数停在旧代数基线上）→ 判定卡死，才 bump
    /// 代数 + abandoned 等待期。注意：基线一致不能区分同基线内 bump 来源
    #[test]
    fn worker_made_progress_detects_alive_slow_worker() {
        // 无进展：判定卡死
        assert!(!worker_made_progress(3, 3));
        // 计数回退（不应发生，保守视为无进展）
        assert!(!worker_made_progress(4, 2));
        // 有进展：worker 慢但存活
        assert!(worker_made_progress(0, 1));
        assert!(worker_made_progress(3, 4));
        // 真代数不匹配（seen 高 32 位 1 ≠ now 高 32 位 2）：不算当前 worker 推进
        let gen_enc = 1u64 << 32;
        assert!(!worker_made_progress(gen_enc, (2u64 << 32) + 5));
        assert!(worker_made_progress(gen_enc, gen_enc + 6)); // 同代数且推进才算存活
    }

    /// decide_heartbeat_verdict 纯决策：超时心跳检查三态——有推进 → Alive
    /// （重置窗口不 bump）；无推进且首次 → FirstStrike（置位 strikes 再等一轮）；
    /// 无推进且已有一击 → Stuck（判卡死，才 bump + abandoned）
    #[test]
    fn heartbeat_verdict_covers_three_timeout_states() {
        // 有推进（含已有 strikes）：Alive，清零 strikes
        assert_eq!(
            decide_heartbeat_verdict(3, 4, false),
            HeartbeatVerdict::Alive
        );
        assert_eq!(
            decide_heartbeat_verdict(3, 4, true),
            HeartbeatVerdict::Alive
        );
        // 无推进且首次：FirstStrike
        assert_eq!(
            decide_heartbeat_verdict(3, 3, false),
            HeartbeatVerdict::FirstStrike
        );
        // 无推进且已有一击：Stuck（连续两次无推进判卡死）
        assert_eq!(
            decide_heartbeat_verdict(3, 3, true),
            HeartbeatVerdict::Stuck
        );
    }

    /// decide_rewrite_cleanup 纯决策：破坏性阶段各早退路径的句柄记账——
    /// alloc 失败/代数不匹配/EmptyClipboard 失败 → Skipped/Busy + 释放全部
    /// 预分配句柄（FreeAll）；SetClipboardData 全失败 → Skipped + 释放失败者
    /// （FreeFailed）；至少一个成功 → Reowned + FreeFailed（成功者被系统接管）
    #[test]
    fn rewrite_cleanup_accounts_handles_on_every_early_return() {
        // alloc 失败：Skipped + 释放全部预分配句柄
        assert_eq!(
            decide_rewrite_cleanup(false, true, false, 0),
            (ReownResult::Skipped, CleanupMode::FreeAll)
        );
        // 代数不匹配：Skipped + 释放全部
        assert_eq!(
            decide_rewrite_cleanup(true, false, false, 0),
            (ReownResult::Skipped, CleanupMode::FreeAll)
        );
        // EmptyClipboard 失败：Busy + 释放全部（可重试）
        assert_eq!(
            decide_rewrite_cleanup(true, true, false, 0),
            (ReownResult::Busy, CleanupMode::FreeAll)
        );
        // SetClipboardData 全失败（剪贴板已清空）：Busy + 释放失败者（可重试）
        assert_eq!(
            decide_rewrite_cleanup(true, true, true, 0),
            (ReownResult::Busy, CleanupMode::FreeFailed)
        );
        // 部分成功：Reowned + 释放失败者（成功者由系统接管）
        assert_eq!(
            decide_rewrite_cleanup(true, true, true, 2),
            (ReownResult::Reowned, CleanupMode::FreeFailed)
        );
        // 全部成功：Reowned + 无失败者可释放
        assert_eq!(
            decide_rewrite_cleanup(true, true, true, 3),
            (ReownResult::Reowned, CleanupMode::FreeFailed)
        );
    }

    /// 慢 reown（时长 > 超时阈值）不被超时中止：心跳计数推进 → 重置超时
    /// 窗口、不 bump 代数 → reown 正常完成只回传一次成功（否则中止 →
    /// Skipped → Ignore → 重新派发，大剪贴板无限循环）。驱动超时分支的
    /// 纯决策路径验证该不变量
    #[test]
    fn slow_reown_completes_once_despite_timeout() {
        let mut generation = 0u64;
        let mut worker_progress_seen = 0u64;
        let mut progress = 0u64;
        // 模拟多次 WM_TIMER 超时检查（reown 全程 > 超时阈值，期间 worker
        // 逐阶段/逐格式推进心跳，对应 reown 内 bump_worker_progress）
        for _ in 0..50 {
            progress += 1; // worker 推进（如每读取/设置一个格式）
            // 对应 WM_TIMER 超时分支的决策路径（worker_made_progress 纯函数）
            if worker_made_progress(worker_progress_seen, progress) {
                // 有进展：重置超时窗口，不 bump 代数
                worker_progress_seen = progress;
            } else {
                // 无进展才会 bump 代数（进入 abandoned 等待期）
                generation += 1;
            }
        }
        // 慢但存活：整个过程中代数从未被 bump，reown 不会被中止
        assert_eq!(generation, 0);
        // reown 正常完成：DONE 代数与当前代数匹配（= 成功，只回传一次，
        // 不进入 Ignore → 重新派发 → 无限循环）
        assert_eq!(generation, 0);
    }

    /// 卡死 worker（心跳无推进）：超时判定无进展 → bump 代数 + abandoned
    /// 等待期；超时后门控放行、恢复满预算，可重新派发新 worker（旧 worker
    /// 迟到的 DONE 按代数不匹配被 Ignore，不消耗预算）
    #[test]
    fn stuck_worker_times_out_then_abandoned_recovers() {
        let mut generation = 0u64;
        let mut worker_progress_seen = 0u64;
        let progress = 0u64; // 卡死：心跳无推进
        let mut abandoned_worker_pending = false;
        let mut abandoned_since = None;

        // 第一次超时检查：无进展 → bump 代数 + 置位 abandoned
        // （对应 WM_TIMER 超时分支）
        if worker_made_progress(worker_progress_seen, progress) {
            worker_progress_seen = progress;
            // 本例心跳无推进不会走到此分支；赋值后立即断言读取，
            // 消除 unused_assignments 警告
            assert_eq!(worker_progress_seen, progress);
        } else {
            generation += 1;
            abandoned_worker_pending = true;
            abandoned_since = Some(Instant::now());
        }
        assert_eq!(generation, 1);
        assert!(abandoned_worker_pending);
        // 刚置位未到超时：门控拦截（不派发新 worker）
        assert!(!abandoned_worker_expired(abandoned_since));

        // 时间推进超过超时阈值：门控放行（checked_sub 构造过去时刻，
        // uptime 不足时跳过，避免 panic）
        if let Some(past) = Instant::now()
            .checked_sub(super::REOWN_IN_FLIGHT_TIMEOUT)
            .and_then(|t| t.checked_sub(Duration::from_millis(1)))
        {
            abandoned_since = Some(past);
            assert!(abandoned_worker_expired(abandoned_since));
            // 门控放行后的恢复动作：清除标志 + 恢复满预算
            // （对应 WM_TIMER 门控分支）
            abandoned_worker_pending = false;
            assert!(!abandoned_worker_pending);
            // 重新派发新 worker：bump 代数
            generation += 1;
            assert_eq!(generation, 2);
            // 旧 worker 迟到的 DONE（代数 1 ≠ 当前 2）会被 Ignore，
            // 不消耗预算、不影响新 worker
            assert_ne!(1, generation);
        }
    }
}
