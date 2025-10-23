document.addEventListener(
  "DOMContentLoaded",
  function () {
    addListener2Video();
  },
  false,
);

document.addEventListener("fullscreenchange", function () {
  if (document.fullscreenElement) {
    fullscreenChanged(true);
  } else {
    fullscreenChanged(false);
  }
});

function addListener2Video() {
  document.querySelectorAll("video").forEach(function (video) {
    video.addEventListener("leavepictureinpicture", function () {
      leavePictureInPicture();
    });
  });
}

function fullscreenChanged(isFullscreen) {
  window.__TAURI_INTERNALS__.invoke("fullscreen_changed", { isFullscreen });
}

function leavePictureInPicture() {
  window.__TAURI_INTERNALS__.invoke("leave_picture_in_picture");
}
