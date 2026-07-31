(function () {
  const BING_URL = "https://cn.bing.com/?form=SPHPRE1&bbtnfrm=";

  // 只处理顶层框架（iframe 的会话历史独立，不影响顶层镜像）
  if (window.self != window.top) {
    return;
  }

  function webviewIpcInvoke(cmd, payload = {}) {
    window.__TAURI_INTERNALS__.invoke(cmd, payload, {
      donotUseCustomProtocol: true,
    });
  }

  // Navigation API（Chromium 102+ / WebView2）可提供权威完整会话历史，
  // 镜像改为"每次条目变化全量上报快照 + 后端对账"，不再靠事件钩子推断。
  var useNavigationApi =
    window.navigation && typeof window.navigation.entries === "function";

  if (useNavigationApi) {
    // 同一文档内已上报的快照签名（index + 条目 key 序列）：
    // DOMContentLoaded / pageshow / currententrychange 会重复触发，内容未变时跳过
    var lastSnapshotKey = null;
    function reportSnapshot() {
      var entries = [];
      try {
        var all = window.navigation.entries();
        for (var i = 0; i < all.length; i++) {
          entries.push({ key: all[i].key, url: all[i].url });
        }
      } catch (e) {
        return; // 快照不可用时放弃本次上报，等待下一次事件
      }
      var index = window.navigation.currentEntry.index;
      var key = index + "|" + entries.map(function (e) { return e.key; }).join(",");
      if (key === lastSnapshotKey) {
        return;
      }
      lastSnapshotKey = key;
      webviewIpcInvoke("history_snapshot", {
        index: index,
        entries: entries,
      });
    }

    // 初始文档加载、bfcache 恢复、push/replace/前进后退/hash 跳转均覆盖
    window.addEventListener("DOMContentLoaded", reportSnapshot, false);
    window.addEventListener("pageshow", reportSnapshot, false);
    window.navigation.addEventListener("currententrychange", reportSnapshot);

    // replaceState 不触发 navigate 事件，只能保留钩子用于 bing 对抗；
    // 镜像对账由快照负责，钩子不再回传状态
    history.replaceState = (function (f) {
      return function replaceState() {
        if (arguments[2] === BING_URL) {
          // 对抗bing首页推广逻辑
          return;
        }
        return f.apply(this, arguments);
      };
    })(history.replaceState);

    return;
  }

  // ---- 降级路径：无 Navigation API 时沿用事件钩子 ----

  history.pushState = (function (f) {
    return function pushState() {
      var ret = f.apply(this, arguments);
      pushHistoryState(arguments[2]);
      return ret;
    };
  })(history.pushState);

  history.replaceState = (function (f) {
    return function replaceState() {
      if (arguments[2] === BING_URL) {
        // 对抗bing首页推广逻辑
        return;
      }
      var ret = f.apply(this, arguments);
      replaceHistoryState(arguments[2]);
      return ret;
    };
  })(history.replaceState);

  // 拦截 location.replace
  var _replace = location.replace.bind(location);
  location.replace = function (url) {
    replaceHistoryState(url);
    return _replace(url);
  };

  window.addEventListener(
    "popstate",
    function () {
      popHistoryState();
    },
    false,
  );

  window.addEventListener(
    "hashchange",
    function () {
      hashChanged();
    },
    false,
  );

  window.addEventListener(
    "DOMContentLoaded",
    function () {
      contentLoaded();
    },
    false,
  );

  function contentLoaded() {
    webviewIpcInvoke("content_loaded", {
      url: window.location.href,
      iconUrl: getIcon(),
      length: history.length,
    });
  }

  function pushHistoryState(url) {
    if (url) {
      url = new URL(url, window.location.href).href;
    } else {
      url = window.location.href;
    }
    webviewIpcInvoke("push_history_state", {
      url,
      length: history.length,
    });
  }

  function replaceHistoryState(url) {
    if (url) {
      url = new URL(url, window.location.href).href;
    } else {
      url = window.location.href;
    }
    webviewIpcInvoke("replace_history_state", {
      url,
      length: history.length,
    });
  }

  function popHistoryState() {
    webviewIpcInvoke("pop_history_state", {
      url: window.location.href,
      length: history.length,
    });
  }

  function hashChanged() {
    webviewIpcInvoke("hash_changed", {
      url: window.location.href,
      length: history.length,
    });
  }

  function getIcon() {
    if (!window.location.href.startsWith("http")) {
      return "";
    }

    var iconUrl = "/favicon.ico";
    // 检查link标签
    var link = document.head.querySelector(
      'link[rel="shortcut icon"],link[rel="icon shortcut"],link[rel="icon"]',
    );
    if (link) {
      iconUrl = link.href;
    }

    return new URL(iconUrl, window.location.href).href;
  }
})();
