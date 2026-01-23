use log::info;
use tauri::{
    Runtime,
    plugin::{Builder, TauriPlugin},
};
use webview2_com::{
    AcceleratorKeyPressedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, COREWEBVIEW2_PHYSICAL_KEY_STATUS,
    },
};

// 虚拟键码常量定义
const VK_LEFT: u32 = 0x25; // 左箭头
const VK_RIGHT: u32 = 0x27; // 右箭头

pub fn prevent_default_hotkey<R: Runtime>() -> TauriPlugin<R> {
    let mut builder = Builder::new("prevent-default-hotkey");

    builder = builder.on_webview_ready(move |webview| {
        let _ = webview.with_webview(|webview| unsafe {
            let mut token: i64 = 0;
            let event_handler =
                AcceleratorKeyPressedEventHandler::create(Box::new(|_sender, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };

                    let mut key_event_type = COREWEBVIEW2_KEY_EVENT_KIND::default();
                    args.KeyEventKind(&mut key_event_type)?;

                    info!("按键拦截1: {key_event_type:?}");
                    if key_event_type != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                        && key_event_type != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                    {
                        return Ok(());
                    }

                    let mut virtual_key = 0;
                    args.VirtualKey(&mut virtual_key)?;

                    let mut physical_key_status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
                    args.PhysicalKeyStatus(&mut physical_key_status)?;

                    if physical_key_status.IsMenuKeyDown.as_bool()
                        && (VK_LEFT == virtual_key || VK_RIGHT == virtual_key)
                    {
                        args.SetHandled(true)?;
                    }

                    Ok(())
                }));

            let _ = webview
                .controller()
                .add_AcceleratorKeyPressed(&event_handler, &mut token);
        });
    });

    builder.build()
}
