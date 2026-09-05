// 浮动 Tab 注入脚本：标题栏 + 控制按钮
// 由 Browser::open_floating_tab 通过 initialization_script 注入
(function () {
  if (window.self !== window.top) return;

  // —— 注入样式（跟随系统主题） ——
  var prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  var iconColor = prefersDark ? "#e0e0e0" : "#1a1a1a";
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
    "  padding: 8px 12px !important;" +
    "  background: " +
    (prefersDark ? "#1d232a" : "#ffffff") +
    " !important;" +
    "  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;" +
    "  font-size: 12px;" +
    "  color: " +
    iconColor +
    " !important;" +
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
    "  border: none !important;" +
    "  border-radius: 6px;" +
    "  background: transparent !important;" +
    "  color: " +
    iconColor +
    " !important;" +
    "  cursor: pointer;" +
    "  display: flex !important;" +
    "  align-items: center;" +
    "  justify-content: center;" +
    "  -webkit-appearance: none !important;" +
    "  appearance: none !important;" +
    "  position: relative !important;" +
    "}" +
    /* 最大化：纯 CSS 方框 */
    "#floating-tab-bar .ft-icon-max {" +
    "  display: inline-block !important;" +
    "  width: 11px !important; height: 11px !important;" +
    "  min-width: 11px !important; min-height: 11px !important;" +
    "  flex-shrink: 0 !important;" +
    "  border: 1.5px solid " +
    iconColor +
    " !important;" +
    "  border-radius: 2px !important;" +
    "  box-sizing: border-box !important;" +
    "}" +
    /* 关闭：纯 CSS 叉号 */
    "#floating-tab-bar .ft-icon-close {" +
    "  display: inline-block !important;" +
    "  width: 12px !important; height: 12px !important;" +
    "  min-width: 12px !important; min-height: 12px !important;" +
    "  flex-shrink: 0 !important;" +
    "  position: relative !important;" +
    "}" +
    "#floating-tab-bar .ft-icon-close::before," +
    "#floating-tab-bar .ft-icon-close::after {" +
    "  content: '';" +
    "  position: absolute;" +
    "  top: 50%; left: 50%;" +
    "  width: 12px; height: 1.5px;" +
    "  background: " +
    iconColor +
    " !important;" +
    "  border-radius: 1px;" +
    "}" +
    "#floating-tab-bar .ft-icon-close::before {" +
    "  transform: translate(-50%, -50%) rotate(45deg);" +
    "}" +
    "#floating-tab-bar .ft-icon-close::after {" +
    "  transform: translate(-50%, -50%) rotate(-45deg);" +
    "}" +
    "#floating-tab-bar .ft-btn:hover {" +
    "  background: " +
    (prefersDark ? "rgba(255,255,255,0.15)" : "rgba(0,0,0,0.08)") +
    " !important;" +
    "}" +
    "#floating-tab-bar .ft-btn.ft-close:hover {" +
    "  background: #e81123 !important;" +
    "}" +
    "#floating-tab-bar .ft-btn.ft-close:hover .ft-icon-close::before," +
    "#floating-tab-bar .ft-btn.ft-close:hover .ft-icon-close::after {" +
    "  background: #fff !important;" +
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
    '    <span class="ft-icon-max"></span>' +
    "  </button>" +
    '  <button class="ft-btn ft-close" title="关闭">' +
    '    <span class="ft-icon-close"></span>' +
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

  var adjustTimer = null;
  var adjustedElems = [];

  // 标题栏高度自动调节：按实际高度同步页面内容的顶部偏移，
  // 同时将页面中 top≈0 的 fixed/sticky 元素下移，避免被标题栏遮挡
  function syncBodyOffset() {
    var rect = bar.getBoundingClientRect();
    var offset = rect.top + rect.height;
    var el = document.documentElement;
    el.style.setProperty("--ft-body-offset", offset + "px");
    el.style.setProperty("padding-top", offset + "px", "important");
    el.style.setProperty("scroll-padding-top", offset + "px", "important");

    // ---- 读阶段：收集需要调整的 fixed/sticky 元素 ----
    var all = document.querySelectorAll("*");
    var targets = [];
    for (var i = 0; i < all.length; i++) {
      var node = all[i];
      if (node === bar || node.contains(bar)) continue;
      var cs = window.getComputedStyle(node);
      var pos = cs.position;
      if (pos !== "fixed" && pos !== "sticky") continue;
      var top = parseFloat(cs.top);
      if (isNaN(top) || top > offset) continue;
      if (!node.dataset.ftElemOrigTop) {
        node.dataset.ftElemOrigTop = String(top);
      }
      targets.push(node);
    }

    // 恢复不再属于目标集合的元素
    for (var m = 0; m < adjustedElems.length; m++) {
      var prev = adjustedElems[m];
      if (targets.indexOf(prev) === -1) {
        prev.style.removeProperty("transition");
        if (prev.dataset.ftElemOrigTop) {
          prev.style.setProperty(
            "top",
            prev.dataset.ftElemOrigTop + "px",
            "important",
          );
          delete prev.dataset.ftElemOrigTop;
        } else {
          prev.style.removeProperty("top");
        }
      }
    }

    // ---- 写阶段：批量应用 top，临时禁用目标元素的 transition ----
    for (var j = 0; j < targets.length; j++) {
      var t = targets[j];
      t.style.setProperty("transition", "none", "important");
      t.style.setProperty(
        "top",
        parseFloat(t.dataset.ftElemOrigTop) + offset + "px",
        "important",
      );
    }
    adjustedElems = targets;

    // 清除上一次定时器，避免竞态
    if (adjustTimer) clearTimeout(adjustTimer);
    if (targets.length) {
      adjustTimer = setTimeout(function () {
        adjustTimer = null;
        for (var k = 0; k < adjustedElems.length; k++) {
          adjustedElems[k].style.removeProperty("transition");
        }
        adjustedElems = [];
      }, 60);
    }
  }

  // —— 按钮事件 ——
  bar.querySelector(".ft-promote").addEventListener("click", function (e) {
    e.stopPropagation();
    ipc("promote_floating_tab");
  });

  var resizeObs = null;
  var domObs = null;
  var titleObs = null;
  var syncRaf = 0;

  function scheduleSyncBodyOffset() {
    if (syncRaf) return;
    syncRaf = requestAnimationFrame(function () {
      syncRaf = 0;
      syncBodyOffset();
    });
  }

  function cleanup() {
    if (resizeObs) {
      resizeObs.disconnect();
      resizeObs = null;
    }
    if (domObs) {
      domObs.disconnect();
      domObs = null;
    }
    if (titleObs) {
      titleObs.disconnect();
      titleObs = null;
    }
    if (syncRaf) {
      cancelAnimationFrame(syncRaf);
      syncRaf = 0;
    }
    if (adjustTimer) {
      clearTimeout(adjustTimer);
      adjustTimer = null;
    }

    var el = document.documentElement;
    el.style.removeProperty("--ft-body-offset");
    el.style.removeProperty("padding-top");
    el.style.removeProperty("scroll-padding-top");

    for (var i = 0; i < adjustedElems.length; i++) {
      var node = adjustedElems[i];
      node.style.removeProperty("transition");
      if (node.dataset.ftElemOrigTop) {
        node.style.setProperty(
          "top",
          node.dataset.ftElemOrigTop + "px",
          "important",
        );
        delete node.dataset.ftElemOrigTop;
      } else {
        node.style.removeProperty("top");
      }
    }
    adjustedElems = [];
  }

  bar.querySelector(".ft-close").addEventListener("click", function (e) {
    e.stopPropagation();
    cleanup();
    ipc("close_floating_tab");
  });

  // —— 注入时机 ——
  function inject() {
    if (document.getElementById("floating-tab-bar")) return;
    // 网页加载失败时 document.body 可能不存在，fallback 到 documentElement
    var container = document.body || document.documentElement;
    if (!container) return;
    document.documentElement.appendChild(style);
    container.appendChild(bar);
    updateTitle();
    syncBodyOffset();
    resizeObs = new ResizeObserver(scheduleSyncBodyOffset);
    resizeObs.observe(bar);
    // 检测页面动态新增的 fixed/sticky 元素（rAF 防抖，合并每帧多次 DOM 变更）
    domObs = new MutationObserver(function (mutations) {
      for (var i = 0; i < mutations.length; i++) {
        var mut = mutations[i];
        if (!mut.addedNodes.length) continue;
        if (mut.target === bar || bar.contains(mut.target)) continue;
        scheduleSyncBodyOffset();
        break;
      }
    });
    domObs.observe(container, { childList: true, subtree: true });
    titleObs = new MutationObserver(updateTitle);
    titleObs.observe(document.querySelector("title") || document.head, {
      childList: true,
      subtree: true,
      characterData: true,
    });
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
      cleanup();
      ipc("close_floating_tab");
    }
  });

  // pageshow（bfcache 恢复时）
  window.addEventListener("pageshow", updateTitle);
})();
