use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use publicsuffix::{List, Psl as _};
use url::{Host, Url};

const ALLOWED_SCHEMES: [&str; 5] = ["http", "https", "file", "data", "ftp"];

/// 参考：https://url.spec.whatwg.org/#fragment-percent-encode-set
const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');

pub fn encode(keyword: &str) -> String {
    utf8_percent_encode(keyword, FRAGMENT).to_string()
}

pub async fn parse_keyword(public_suffix: Option<List>, keyword: &str) -> Option<Url> {
    let input = keyword.trim();
    if input.is_empty() {
        return None;
    }

    if (input.contains("\\") || input.contains("/"))
        && let Ok(true) = tokio::fs::try_exists(input).await
    {
        // Windows 文件路径会被误解为URL
        return Url::parse(&format!("file:///{input}")).ok();
    }

    // 1. 尝试直接解析为URL
    if let Ok(url) = Url::parse(input)
        && ALLOWED_SCHEMES.contains(&url.scheme())
    {
        return Some(url);
    }

    // 3. 尝试补全协议并解析URL
    let Ok(mut url) = Url::parse(&format!("https://{}", input)) else {
        return complete_search_url(input);
    };

    let Some(host) = url.host() else {
        return complete_search_url(input);
    };
    let Host::Domain(host) = host else {
        // ip host
        url.set_scheme("http").unwrap();
        return Some(url);
    };

    if host.eq_ignore_ascii_case("localhost") {
        return Some(url);
    }

    if let Some(public_suffix) = public_suffix {
        let Some(suffix) = public_suffix.suffix(host.as_bytes()) else {
            return Some(url);
        };
        if suffix.typ().is_some() {
            return Some(url);
        }
    } else if host.split('.').next_back().is_some() {
        return Some(url);
    }

    // 5. 其他情况视为搜索
    complete_search_url(input)
}

/// TODO 配置化
fn complete_search_url(input: &str) -> Option<Url> {
    Url::parse_with_params("https://cn.bing.com/search", &[("q", input)]).ok()
}
