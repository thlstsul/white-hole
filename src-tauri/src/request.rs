use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::FetchError;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct HttpHeader {
    pub key: String,
    pub value: String,
}

/// fetch 函数的可选配置项，支持 JSON 序列化
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchOptions {
    pub method: Option<String>,
    pub headers: Option<Vec<HttpHeader>>,
    pub body: Option<String>,
}

/// 自定义的响应类型，完全独立于 reqwest，支持 JSON 序列化
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    #[serde(with = "time::serde::iso8601")]
    pub done_date: OffsetDateTime,
    pub status: u16,
    pub headers: Vec<HttpHeader>,
    pub body: Vec<u8>,
    pub elapsed_time: i32,
}

/// 核心 fetch 函数，返回一个支持序列化的 Response
pub async fn fetch(url: &str, options: Option<FetchOptions>) -> Result<Response, FetchError> {
    let client = Client::new();

    // 构建请求
    let mut request_builder: RequestBuilder = match options.as_ref().and_then(|o| o.method.as_ref())
    {
        Some(method) => match method.to_uppercase().as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "DELETE" => client.delete(url),
            "HEAD" => client.head(url),
            "PATCH" => client.patch(url),
            _ => client.get(url),
        },
        None => client.get(url),
    };

    // 添加请求头
    if let Some(opts) = &options {
        if let Some(headers) = &opts.headers {
            for HttpHeader { key: name, value } in headers {
                request_builder = request_builder.header(name.as_str(), value);
            }
        }
        // 添加请求体
        if let Some(body) = &opts.body {
            request_builder = request_builder.body(body.clone());
        }
    }

    // 发送请求并获得原始 reqwest::Response
    let done_date = OffsetDateTime::now_local()?;
    let raw_response = request_builder.send().await?;

    // 提取所有数据，释放 reqwest::Response
    let status = raw_response.status().as_u16();

    // 将头信息转换为简单的键值对列表（保留多值）
    let headers = raw_response
        .headers()
        .iter()
        .flat_map(|(key, value)| {
            // 头值可能不是合法 UTF-8
            let value_str = value.to_str().unwrap_or("...").to_string();
            std::iter::once(HttpHeader {
                key: key.as_str().to_string(),
                value: value_str,
            })
        })
        .collect();

    // 读取完整响应体
    let body = raw_response.bytes().await?.to_vec();

    Ok(Response {
        status,
        headers,
        body,
        done_date,
        elapsed_time: (OffsetDateTime::now_local()? - done_date).whole_milliseconds() as i32,
    })
}
