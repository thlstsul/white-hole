const BING_URL = "https://cn.bing.com/?form=SPHPRE1&bbtnserp=1&bbtnfrm=";

if (window.self == window.top) {
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
}

function contentLoaded() {
  window.__TAURI_INTERNALS__.invoke("content_loaded", {
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
  window.__TAURI_INTERNALS__.invoke("push_history_state", {
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
  window.__TAURI_INTERNALS__.invoke("replace_history_state", {
    url,
    length: history.length,
  });
}

function popHistoryState() {
  window.__TAURI_INTERNALS__.invoke("pop_history_state");
}

function hashChanged() {
  window.__TAURI_INTERNALS__.invoke("hash_changed", {
    url: window.location.href,
    length: history.length,
  });
}

function getIcon() {
  var iconUrl = "/favicon.ico";
  // 检查link标签
  var link = document.head.querySelector(
    'link[rel="shortcut icon"],link[rel="icon"]',
  );
  if (link) {
    iconUrl = link.href;
  }

  return new URL(iconUrl, window.location.href).href;
}
