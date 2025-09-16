use std::borrow::Cow;

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

pub const USER_AGENT: &str = "USER_AGENT";

pub fn setup_user_agent() {
    unsafe {
        std::env::set_var(
            USER_AGENT,
            UserAgentPlatform::default()
                .to_user_agent_string()
                .to_string(),
        );
    }
}

pub fn get_user_agent() -> String {
    std::env::var(USER_AGENT).unwrap_or_default()
}

pub enum UserAgentPlatform {
    Desktop,
    Android,
    OpenHarmony,
    Ios,
}

impl UserAgentPlatform {
    /// Return the default `UserAgentPlatform` for this platform. This is
    /// not an implementation of `Default` so that it can be `const`.
    pub const fn default() -> Self {
        if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_env = "ohos") {
            Self::OpenHarmony
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else {
            Self::Desktop
        }
    }

    /// Convert this [`UserAgentPlatform`] into its corresponding user-agent string.
    ///
    /// Returns a [`Cow<'static, str>`] to avoid unnecessary allocations when
    /// the user-agent string is known at compile time.
    pub fn to_user_agent_string(&self) -> Cow<'static, str> {
        let base_ua = match self {
            UserAgentPlatform::Desktop => Self::desktop_user_agent(),
            UserAgentPlatform::Android => Cow::Borrowed(
                "Mozilla/5.0 (Android; Mobile) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 ",
            ),
            UserAgentPlatform::OpenHarmony => Cow::Borrowed(
                "Mozilla/5.0 (OpenHarmony; Mobile) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0",
            ),
            UserAgentPlatform::Ios => Cow::Borrowed(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/537.36 (KHTML, like Gecko)",
            ),
        };

        Cow::Owned(format!("{} {}", base_ua, APP_USER_AGENT))
    }

    /// Generate the appropriate user-agent string for desktop platforms.
    fn desktop_user_agent() -> Cow<'static, str> {
        if cfg!(target_os = "windows") {
            let architecture = if cfg!(target_arch = "x86_64") {
                "x64"
            } else {
                // Windows on non-x86_64 architectures
                ""
            };

            Cow::Owned(format!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; {}) AppleWebKit/537.36 (KHTML, like Gecko) Edg/139.0.0.0",
                architecture
            ))
        } else if cfg!(target_os = "macos") {
            Cow::Borrowed(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15) AppleWebKit/537.36 (KHTML, like Gecko)",
            )
        } else {
            // Default to Linux-style for other desktop platforms
            let architecture = if cfg!(target_arch = "x86_64") {
                "x86_64"
            } else if cfg!(target_arch = "x86") {
                "i686"
            } else {
                // For other architectures (ARM, RISC-V, etc.), use the actual target architecture
                std::env::consts::ARCH
            };

            Cow::Owned(format!(
                "Mozilla/5.0 (X11; Linux {}) AppleWebKit/537.36 (KHTML, like Gecko)",
                architecture
            ))
        }
    }
}
