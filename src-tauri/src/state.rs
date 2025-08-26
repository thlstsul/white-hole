use serde::Serialize;
use tauri::async_runtime::RwLock;

#[derive(Debug, Clone, Serialize)]
pub struct BrowserState {
    pub icon_url: String,
    pub title: String,
    pub url: String,
    pub maximized: bool,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub focus: bool,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            title: "白洞".to_string(),
            url: "White Hole".to_string(),
            icon_url: String::new(),
            maximized: false,
            loading: false,
            can_back: false,
            can_forward: false,
            focus: false,
        }
    }
}

pub struct Focused(RwLock<bool>);

impl Focused {
    pub fn new() -> Self {
        Self(RwLock::new(false))
    }

    pub async fn set(&self, value: bool) -> bool {
        let mut focus = self.0.write().await;
        if *focus == value {
            return false;
        }

        *focus = value;
        true
    }

    pub async fn get(&self) -> bool {
        *self.0.read().await
    }
}
