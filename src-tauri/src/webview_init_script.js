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
    if (e.altKey && (e.code === "ArrowLeft" || e.code === "ArrowRight")) {
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
  invoke("icon_changed", { iconUrl });
}

function pushHistoryState() {
  invoke("push_history_state");
}

function replaceHistoryState() {
  invoke("replace_history_state");
}

function hashChanged() {
  invoke("hash_changed");
}

function fullscreenChanged(isFullscreen) {
  invoke("fullscreen_changed", { isFullscreen });
}

function leavePictureInPicture() {
  invoke("leave_picture_in_picture");
}

function invoke(cmd, payload) {
  window.__TAURI_INTERNALS__.invoke(cmd, payload);
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
