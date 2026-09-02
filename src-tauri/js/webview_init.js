(function () {
  // 只处理顶层框架（iframe 的会话历史独立，不影响顶层镜像）
  if (window.self != window.top) {
    return;
  }

  function webviewIpcInvoke(cmd, payload = {}) {
    window.__TAURI_INTERNALS__
      .invoke(cmd, payload, {
        donotUseCustomProtocol: true,
      })
      .catch(function () {}); // 命令失败由后端日志记录，前端静默
  }

  // Navigation API（Chromium 102+ / WebView2）提供权威完整会话历史，
  // 镜像策略：每次条目变化全量上报快照 + 后端对账
  // 同一文档内已上报的快照签名（index + 当前 URL + 条目 key 序列）：
  // DOMContentLoaded / pageshow / currententrychange 会重复触发，内容未变时跳过；
  // URL 入签使 replaceState（只改 URL 不改 key）不会被去重误吞
  let lastSnapshotKey = null;
  function reportSnapshot() {
    let all;
    try {
      all = window.navigation.entries();
    } catch (e) {
      return; // 快照不可用时放弃本次上报，等待下一次事件
    }
    const current = window.navigation.currentEntry;
    // currentEntry 在导航条目尚未建立时为 null（早期 DOMContentLoaded、
    // 无历史条目的文档）；条目为空同样算不出可靠 index：
    // 放弃本次上报，等待 pageshow / currententrychange 重试
    if (!current || all.length === 0) {
      return;
    }
    // 对抗 bing 首页推广：整条移除推广条目（不清洗成干净 URL 保留）。
    // index 重算为当前条目在过滤后列表中的位置；若当前条目本身就是推广
    // 条目（已被过滤），index 指向其前一个保留条目，镜像中无推广记录。
    const entries = [];
    const keys = [];
    let index = 0;
    for (let i = 0; i < all.length; i++) {
      if (isBingPromoUrl(all[i].url)) {
        continue;
      }
      entries.push({ key: all[i].key, url: all[i].url });
      keys.push(all[i].key);
      if (i <= current.index) {
        index++;
      }
    }
    index = Math.max(0, index - 1);
    const key = current.index + "|" + current.url + "|" + keys.join(",");
    if (key === lastSnapshotKey) {
      return;
    }
    lastSnapshotKey = key;
    webviewIpcInvoke("history_snapshot", {
      index,
      entries,
    });
  }

  // 初始文档加载、bfcache 恢复、push/replace/前进后退/hash 跳转均覆盖；
  // DOMContentLoaded 额外补 content_loaded（快照不含图标，需单独上报）
  window.addEventListener(
    "DOMContentLoaded",
    function () {
      reportSnapshot();
      contentLoaded();
    },
    false,
  );
  window.addEventListener("pageshow", reportSnapshot, false);

  if (window.navigation) {
    // Navigation API（Chromium 102+ / WebView2）提供权威完整会话历史
    window.navigation.addEventListener("currententrychange", reportSnapshot);
  } else {
    // 非 Chromium 引擎（macOS WKWebView / Linux WebKitGTK）没有 Navigation API：
    // 退化为 popstate / hashchange / 历史 API 包装兜底上报，保证同文档导航
    // （前进后退、hash 变化、SPA pushState）能更新镜像中的当前 URL 与导航记录
    window.addEventListener("popstate", reportFallback, false);
    window.addEventListener("hashchange", reportFallback, false);
    // pushState/replaceState 不触发 popstate/hashchange，SPA 用历史 API
    // 跳转时需自行触发兜底上报（在页面脚本执行前包装，尽力拦截）
    const wrapHistoryMethod = function (method) {
      const original = history[method];
      history[method] = function () {
        const result = original.apply(this, arguments);
        reportFallback();
        return result;
      };
    };
    wrapHistoryMethod("pushState");
    wrapHistoryMethod("replaceState");
  }

  function contentLoaded() {
    webviewIpcInvoke("content_loaded", {
      url: window.location.href,
      iconUrl: getIcon(),
      length: history.length,
    });
  }

  // 无 Navigation API 时的兜底：把当前 URL 按内容加载上报。
  // 按上次上报的 URL 去重：back/forward/hash 反复经过同一 URL 时不再重复
  // 上报，避免 content_loaded 被后端当作整页加载而虚增访问计数
  let lastFallbackUrl = null;
  function reportFallback() {
    if (window.location.href === lastFallbackUrl) {
      return;
    }
    lastFallbackUrl = window.location.href;
    contentLoaded();
  }

  function getIcon() {
    if (!window.location.href.startsWith("http")) {
      return "";
    }

    let iconUrl = "/favicon.ico";
    // 检查link标签
    const link = document.head.querySelector(
      'link[rel="shortcut icon"],link[rel="icon shortcut"],link[rel="icon"]',
    );
    if (link) {
      iconUrl = link.href;
    }

    return new URL(iconUrl, window.location.href).href;
  }

  // ===== bing 首页推广对抗 =====
  // bing 首页在用户交互时会把地址栏改写为带跟踪参数的推广链接
  // （形如 https://cn.bing.com/?form=SPHPRE1&bbtnfrm= ），会污染历史；
  // 推广参数与域名（cn./www.）会变化，不能精确匹配，按 URL 特征识别。
  function isBingPromoUrl(url) {
    if (typeof url !== "string" || url.length === 0) {
      return false;
    }
    let parsed;
    try {
      parsed = new URL(url, window.location.href);
    } catch (e) {
      return false;
    }
    const host = parsed.hostname.toLowerCase();
    if (host !== "bing.com" && !host.endsWith(".bing.com")) {
      return false;
    }
    const path = parsed.pathname;
    if (path !== "/" && path !== "") {
      return false;
    }
    return (
      parsed.searchParams.has("bbtnfrm") ||
      (parsed.searchParams.get("form") || "")
        .toUpperCase()
        .startsWith("SPHPRE1")
    );
  }
})();
