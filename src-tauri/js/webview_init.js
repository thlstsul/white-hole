(function () {
  const BING_URL = "https://cn.bing.com/?form=SPHPRE1&bbtnfrm=";

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
    let entries, keys;
    try {
      const all = window.navigation.entries();
      entries = [];
      keys = [];
      for (let i = 0; i < all.length; i++) {
        entries.push({ key: all[i].key, url: all[i].url });
        keys.push(all[i].key);
      }
    } catch (e) {
      return; // 快照不可用时放弃本次上报，等待下一次事件
    }
    const current = window.navigation.currentEntry;
    const key = current.index + "|" + current.url + "|" + keys.join(",");
    if (key === lastSnapshotKey) {
      return;
    }
    lastSnapshotKey = key;
    webviewIpcInvoke("history_snapshot", {
      index: current.index,
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
  window.navigation.addEventListener("currententrychange", reportSnapshot);

  // replaceState 会触发 currententrychange 已被快照覆盖，不触发 navigate；
  // 此处仅保留钩子用于 bing 对抗
  const origReplaceState = history.replaceState;
  history.replaceState = function () {
    if (arguments[2] === BING_URL) {
      // 对抗bing首页推广逻辑
      return;
    }
    return origReplaceState.apply(this, arguments);
  };

  function contentLoaded() {
    webviewIpcInvoke("content_loaded", {
      url: window.location.href,
      iconUrl: getIcon(),
      length: history.length,
    });
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
})();
