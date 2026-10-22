// Small, dependency-free enhancements. Every download link and every command
// works without JavaScript; this only adds tabs, copy buttons and a shadow.

(() => {
  const toast = document.querySelector(".toast");
  let toastTimer;

  function say(message) {
    if (!toast) return;
    toast.textContent = message;
    toast.classList.add("show");
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => toast.classList.remove("show"), 1800);
  }

  // ---------- tabs ----------

  document.querySelectorAll("[data-tabs]").forEach((root) => {
    const tabs = [...root.querySelectorAll('[role="tab"]')];
    if (tabs.length === 0) return;

    const select = (id, focus = false) => {
      tabs.forEach((tab) => {
        const active = tab.dataset.tab === id;
        tab.setAttribute("aria-selected", String(active));
        tab.tabIndex = active ? 0 : -1;
        if (active && focus) tab.focus();

        const panel = document.getElementById(tab.dataset.tab);
        if (panel) panel.hidden = !active;
      });
    };

    tabs.forEach((tab, index) => {
      tab.addEventListener("click", () => select(tab.dataset.tab));

      tab.addEventListener("keydown", (event) => {
        const keys = ["ArrowLeft", "ArrowRight", "Home", "End"];
        if (!keys.includes(event.key)) return;
        event.preventDefault();

        let next = index;
        if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
        if (event.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
        if (event.key === "Home") next = 0;
        if (event.key === "End") next = tabs.length - 1;

        select(tabs[next].dataset.tab, true);
      });
    });

    // Start from the markup's selection, so a reload with a hash cannot leave
    // two panels visible.
    const initial = tabs.find((tab) => tab.getAttribute("aria-selected") === "true") ?? tabs[0];
    select(initial.dataset.tab);
  });

  // ---------- copy ----------

  async function copy(text) {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      return;
    }

    // Fallback for file:// previews and older WebViews.
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    if (!ok) throw new Error("copy failed");
  }

  document.querySelectorAll("[data-copy]").forEach((button) => {
    button.addEventListener("click", async () => {
      const block = button.closest(".code");
      const code = block?.querySelector("code");
      if (!code) return;

      // Keep the shebang-free commands exactly as shown, without trailing
      // whitespace copied from the HTML indentation.
      const text = code.textContent
        .split("\n")
        .map((line) => line.trimEnd())
        .join("\n")
        .trim();

      try {
        await copy(text);
        button.classList.add("is-copied");
        say("Commands copied");
        window.setTimeout(() => button.classList.remove("is-copied"), 1600);
      } catch {
        say("Select the command and copy it manually");
      }
    });
  });

  // ---------- nav shadow ----------

  const nav = document.getElementById("nav");
  if (nav) {
    const onScroll = () => nav.classList.toggle("is-scrolled", window.scrollY > 10);
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
  }
})();
