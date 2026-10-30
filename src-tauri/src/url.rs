//! URL 处理：搜索关键词编码、历史记录的 URL 规范化（取 eTLD+1 等）。

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use publicsuffix::{List, Psl as _};
use tokio::fs::try_exists;
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
        && let Ok(true) = try_exists(input).await
    {
        // 本地存在的路径若按普通 URL 解析，"C:\dir\a.html" 会得到 scheme="c"
        // 的无效地址；先转成 file:/// URL
        return Url::parse(&format!("file:///{input}")).ok();
    }

    if let Ok(url) = Url::parse(input)
        && ALLOWED_SCHEMES.contains(&url.scheme())
    {
        return Some(url);
    }

    let Ok(mut url) = Url::parse(&format!("https://{}", input)) else {
        return complete_search_url(input);
    };

    let Some(host) = url.host() else {
        return complete_search_url(input);
    };
    let Host::Domain(host) = host else {
        let _ = url.set_scheme("http");
        return Some(url);
    };

    if host.eq_ignore_ascii_case("localhost") {
        let _ = url.set_scheme("http");
        return Some(url);
    }

    if !host.contains('.') {
        return complete_search_url(input);
    }

    if let Some(public_suffix) = public_suffix
        && let Some(suffix) = public_suffix.suffix(host.as_bytes())
        && suffix.typ().is_some()
    {
        return Some(url);
    }

    complete_search_url(input)
}

fn complete_search_url(input: &str) -> Option<Url> {
    // TODO: 搜索引擎与模板可配置化（当前硬编码 Bing）
    Url::parse_with_params("https://cn.bing.com/search", &[("q", input)]).ok()
}
