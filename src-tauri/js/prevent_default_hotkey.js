(function () {
  window.addEventListener("keydown", function (e) {
    if (e.altKey && (e.code === "ArrowLeft" || e.code === "ArrowRight")) {
      e.preventDefault();
    } else if ((e.ctrlKey && e.code === "KeyR") || e.code == "F5") {
      e.preventDefault();
    }
  });
})();
