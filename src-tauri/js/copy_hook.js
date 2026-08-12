(function () {
  // WebView2/Chromium 内部复制时，剪贴板所有者是 Chromium 内部窗口，
  // Windows 剪贴板历史（Win+V）无法识别，复制的内容能粘贴但不会出现在历史中。
  // 复制完成后通知宿主重新接管剪贴板（clipboard_reown），让历史服务能记录。
  function webviewIpcInvoke(cmd, payload) {
    window.__TAURI_INTERNALS__
      .invoke(cmd, payload || {}, {
        donotUseCustomProtocol: true,
      })
      .catch(function () {}); // 失败由后端日志记录，前端静默
  }

  function scheduleReown() {
    // 等 WebView2 默认复制动作同步完成后再接管
    setTimeout(function () {
      webviewIpcInvoke("clipboard_reown");
    }, 100);
  }

  // 快捷键 Ctrl+C / Ctrl+X、右键菜单复制、execCommand("copy"/"cut") 都会触发 copy/cut 事件
  window.addEventListener("copy", scheduleReown, false);
  window.addEventListener("cut", scheduleReown, false);

  // 页面主动调用 navigator.clipboard.writeText 的复制（如"复制链接"按钮）
  const clipboardApi = navigator.clipboard;
  if (clipboardApi && typeof clipboardApi.writeText === "function") {
    const origWriteText = clipboardApi.writeText;
    clipboardApi.writeText = function (text) {
      const promise = origWriteText.call(this, text);
      if (promise && typeof promise.then === "function") {
        promise.then(scheduleReown, function () {});
      }
      return promise;
    };
  }
})();
