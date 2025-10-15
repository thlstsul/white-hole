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
      iconChanged(getIcon());
      addListener2Video();
    },
    false,
  );

  window.addEventListener("keydown", function (e) {
    if (e.altKey && e.code === "ArrowLeft") {
      e.preventDefault();
    }
  });
  window.addEventListener("keydown", function (e) {
    if (e.altKey && e.code === "ArrowRight") {
      e.preventDefault();
    }
  });

  document.addEventListener("fullscreenchange", function () {
    if (document.fullscreenElement) {
      fullscreenChanged(true);
    } else {
      fullscreenChanged(false);
    }
  });
}

function iconChanged(iconUrl) {
  window.__TAURI_INTERNALS__.invoke("icon_changed", { iconUrl });
}

function pushHistoryState() {
  window.__TAURI_INTERNALS__.invoke("push_history_state");
}

function replaceHistoryState() {
  window.__TAURI_INTERNALS__.invoke("replace_history_state");
}

function hashChanged() {
  window.__TAURI_INTERNALS__.invoke("hash_changed");
}

function fullscreenChanged(isFullscreen) {
  window.__TAURI_INTERNALS__.invoke("fullscreen_changed", { isFullscreen });
}

function leavePictureInPicture() {
  window.__TAURI_INTERNALS__.invoke("leave_picture_in_picture");
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

function addListener2Video() {
  document.querySelectorAll("video").forEach(function (video) {
    video.addEventListener("leavepictureinpicture", function () {
      leavePictureInPicture();
    });
  });
}
