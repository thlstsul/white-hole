use log::error;
use tauri::{
    Runtime,
    plugin::{Builder, TauriPlugin},
    webview::PlatformWebview,
};
use webview2_com::{
    AcceleratorKeyPressedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, COREWEBVIEW2_PHYSICAL_KEY_STATUS,
    },
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_F5, VK_LEFT, VK_P, VK_R, VK_RIGHT,
};

pub fn prevent_default_hotkey<R: Runtime>() -> TauriPlugin<R> {
    let mut builder = Builder::new("prevent-default-hotkey");

    builder = builder.on_webview_ready(move |webview| {
        if let Err(e) = webview.with_webview(add_accelerator_key_pressed) {
            error!("注册 Webview2 句柄失败：{e}");
        }
    });

    builder.build()
}

fn add_accelerator_key_pressed(webview: PlatformWebview) {
    let mut token: i64 = 0;
    let event_handler =
        AcceleratorKeyPressedEventHandler::create(Box::new(|_sender, args| unsafe {
            let Some(args) = args else {
                return Ok(());
            };

            let mut key_event_type = COREWEBVIEW2_KEY_EVENT_KIND::default();
            args.KeyEventKind(&mut key_event_type)?;
            if key_event_type != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                && key_event_type != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
            {
                return Ok(());
            }

            let mut physical_key_status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
            args.PhysicalKeyStatus(&mut physical_key_status)?;
            if physical_key_status.WasKeyDown.as_bool() {
                return Ok(());
            }

            let mut virtual_key = 0;
            args.VirtualKey(&mut virtual_key)?;
            if (physical_key_status.IsMenuKeyDown.as_bool()
                && matches!(VIRTUAL_KEY(virtual_key as u16), VK_LEFT | VK_RIGHT))
                || matches!(VIRTUAL_KEY(virtual_key as u16), VK_F5)
            {
                args.SetHandled(true)?;
                return Ok(());
            }

            let ctrl = (GetKeyState(VK_CONTROL.0 as i32) as i32 & 0x8000) != 0;
            if ctrl && matches!(VIRTUAL_KEY(virtual_key as u16), VK_R | VK_P) {
                args.SetHandled(true)?;
                return Ok(());
            }

            Ok(())
        }));

    unsafe {
        if let Err(e) = webview
            .controller()
            .add_AcceleratorKeyPressed(&event_handler, &mut token)
        {
            error!("注册快捷键拦截失败：{e}");
        }
    }
}
