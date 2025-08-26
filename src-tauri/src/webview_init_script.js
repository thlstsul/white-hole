if (window.self == window.top) {
  history.pushState = (function (f) {
    return function pushState() {
      var ret = f.apply(this, arguments);
      pushHistoryState();
      return ret;
    };
  })(history.pushState);

  history.replaceState = (function (f) {
    return function replaceState() {
      var ret = f.apply(this, arguments);
      replaceHistoryState();
      return ret;
    };
  })(history.replaceState);

  window.addEventListener("popstate", function () {
    // popstate(hash changed will pop too)
    popHistoryState();
  });

  window.addEventListener(
    "DOMContentLoaded",
    function () {
      iconChanged(getIcon());
    },
    false,
  );
}

function iconChanged(iconUrl) {
  window.__TAURI__.core.invoke("icon_changed", { iconUrl });
}

function pushHistoryState() {
  window.__TAURI__.core.invoke("push_history_state");
}

function replaceHistoryState() {
  window.__TAURI__.core.invoke("replace_history_state");
}

function popHistoryState() {
  window.__TAURI__.core.invoke("pop_history_state");
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
