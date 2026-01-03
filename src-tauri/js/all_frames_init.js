document.addEventListener("fullscreenchange", function () {
  if (document.fullscreenElement) {
    fullscreenChanged(true);
  } else {
    fullscreenChanged(false);
  }
});

document.addEventListener(
  "DOMContentLoaded",
  function () {
    document.querySelectorAll("a").forEach(addListener2Link);
    document.querySelectorAll("video").forEach(addListener2Video);

    var observer = new MutationObserver(function (mutations) {
      mutations.forEach(function (mutation) {
        mutation.addedNodes.forEach(function (node) {
          if (node.nodeType === 1) {
            // 元素节点
            if (node.tagName === "A") {
              addListener2Link(node);
            } else if (node.tagName === "VIDEO") {
              addListener2Video(node);
            }

            // 检查节点内的a标签
            var links = node.querySelectorAll ? node.querySelectorAll("a") : [];
            links.forEach(addListener2Link);
            // 检查节点内的video标签
            var videos = node.querySelectorAll
              ? node.querySelectorAll("video")
              : [];
            videos.forEach(addListener2Video);
          }
        });
      });
    });

    observer.observe(document.body, {
      childList: true,
      subtree: true,
    });
  },
  false,
);

function addListener2Video(video) {
  video.addEventListener("leavepictureinpicture", function () {
    leavePictureInPicture();
  });
}

function addListener2Link(link) {
  var url = link.href;
  if (!url || !url.startsWith("http") || url.endsWith("#")) {
    return;
  }

  link.addEventListener("mouseenter", function () {
    focusLink(url);
  });

  link.addEventListener("mouseleave", function () {
    blurLink();
  });

  link.addEventListener("focus", function () {
    focusLink(url);
  });

  link.addEventListener("blur", function () {
    blurLink();
  });

  link.addEventListener("click", function () {
    clickLink(url);
  });
}

function fullscreenChanged(isFullscreen) {
  window.__TAURI_INTERNALS__.invoke("fullscreen_changed", { isFullscreen });
}

function leavePictureInPicture() {
  window.__TAURI_INTERNALS__.invoke("leave_picture_in_picture");
}

function focusLink(url) {
  window.__TAURI_INTERNALS__.invoke("focus_link", { url });
}

function blurLink() {
  window.__TAURI_INTERNALS__.invoke("blur_link");
}

function clickLink(url) {
  window.__TAURI_INTERNALS__.invoke("click_link", { url });
}
