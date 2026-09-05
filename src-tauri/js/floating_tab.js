// 浮动 Tab 注入脚本：标题栏 + 控制按钮
// 由 Browser::open_floating_tab 通过 initialization_script 注入
(function () {
  if (window.self !== window.top) return;

  var BODY_GAP = 8; // 标题栏底部到页面内容的间距

  // —— 注入样式（跟随系统主题） ——
  var prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  var style = document.createElement("style");
  style.textContent =
    "#floating-tab-bar {" +
    "  position: fixed;" +
    "  top: 0; left: 0; right: 0;" +
    "  z-index: 2147483647;" +
    "  box-sizing: border-box;" +
    "  display: flex;" +
    "  align-items: center;" +
    "  gap: 8px;" +
    "  padding: 8px 12px;" +
    "  background: " + (prefersDark ? "#1d232a" : "#ffffff") + ";" +
    "  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;" +
    "  font-size: 12px;" +
    "  color: " + (prefersDark ? "#e0e0e0" : "#1a1a1a") + ";" +
    "  user-select: none;" +
    "  -webkit-app-region: drag;" +
    "}" +
    "#floating-tab-bar .ft-title {" +
    "  flex: 1;" +
    "  min-width: 0;" +
    "  overflow: hidden;" +
    "  text-overflow: ellipsis;" +
    "  white-space: nowrap;" +
    "}" +
    "#floating-tab-bar .ft-controls {" +
    "  display: flex;" +
    "  gap: 4px;" +
    "  -webkit-app-region: no-drag;" +
    "}" +
    "#floating-tab-bar .ft-btn {" +
    "  width: 24px; height: 24px;" +
    "  border: none; border-radius: 6px;" +
    "  background: transparent;" +
    "  color: " + (prefersDark ? "#e0e0e0" : "#1a1a1a") + ";" +
    "  cursor: pointer;" +
    "  display: flex;" +
    "  align-items: center;" +
    "  justify-content: center;" +
    "}" +
    "#floating-tab-bar .ft-btn:hover {" +
    "  background: " + (prefersDark ? "rgba(255,255,255,0.15)" : "rgba(0,0,0,0.08)") + ";" +
    "}" +
    "#floating-tab-bar .ft-btn.ft-close:hover {" +
    "  background: #e81123; color: #fff;" +
    "}" +
    "html { padding-top: var(--ft-body-offset, 48px) !important; }" +
    "body { margin-top: 0 !important; }" +
    "[id], [name], a[href] { scroll-margin-top: var(--ft-body-offset, 48px) !important; }";

  // —— 注入标题栏 DOM ——
  var bar = document.createElement("div");
  bar.id = "floating-tab-bar";
  bar.innerHTML =
    '<span class="ft-title"></span>' +
    '<div class="ft-controls">' +
    '  <button class="ft-btn ft-promote" title="最大化">' +
    '    <svg width="14" height="14" viewBox="0 0 14 14" fill="none">' +
    '      <rect x="1" y="1" width="12" height="12" rx="2" stroke="currentColor" stroke-width="1.5"/>' +
    "    </svg>" +
    "  </button>" +
    '  <button class="ft-btn ft-close" title="关闭">' +
    '    <svg width="14" height="14" viewBox="0 0 14 14" fill="none">' +
    '      <path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>' +
    "    </svg>" +
    "  </button>" +
    "</div>";

  // —— 工具函数 ——
  function ipc(cmd) {
    window.__TAURI_INTERNALS__
      .invoke(cmd, {}, { donotUseCustomProtocol: true })
      .catch(function () {});
  }

  function updateTitle() {
    var titleEl = bar.querySelector(".ft-title");
    var title = document.title || window.location.href;
    titleEl.textContent = title;
  }

  // 标题栏高度自动调节：按实际高度同步页面内容的顶部偏移
  function syncBodyOffset() {
    var rect = bar.getBoundingClientRect();
    document.documentElement.style.setProperty(
      "--ft-body-offset",
      rect.top + rect.height + BODY_GAP + "px"
    );
  }

  // —— 按钮事件 ——
  bar.querySelector(".ft-promote").addEventListener("click", function (e) {
    e.stopPropagation();
    ipc("promote_floating_tab");
  });
  bar.querySelector(".ft-close").addEventListener("click", function (e) {
    e.stopPropagation();
    ipc("close_floating_tab");
  });

  // —— 注入时机 ——
  function inject() {
    if (document.body && !document.getElementById("floating-tab-bar")) {
      document.documentElement.appendChild(style);
      document.body.appendChild(bar);
      updateTitle();
      syncBodyOffset();
      new ResizeObserver(syncBodyOffset).observe(bar);
      var observer = new MutationObserver(updateTitle);
      observer.observe(document.querySelector("title") || document.head, {
        childList: true,
        subtree: true,
        characterData: true,
      });
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", inject);
  } else {
    inject();
  }

  // —— Esc 关闭浮动 Tab（FT-16）——
  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape") {
      e.stopPropagation();
      ipc("close_floating_tab");
    }
  });

  // pageshow（bfcache 恢复时）
  window.addEventListener("pageshow", updateTitle);
})();