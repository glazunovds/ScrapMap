/**
 * Interface strings, keyed and looked up at runtime.
 *
 * English is the default and the fallback. A language is a file in `locales/`
 * plus an entry in `LANGUAGES` below, so adding one needs no code changes
 * elsewhere; a key missing from a translation falls back to English rather than
 * rendering blank, which keeps a half-finished translation usable.
 *
 * **Only mark elements whose text is static.** Applying a dictionary rewrites
 * every `data-i18n` element, so putting one on something the code writes at
 * runtime overwrites the live value with the initial one -- and because the
 * dictionary loads asynchronously it wins that race. The profile summary sat
 * permanently on "Working out which map profile this is…" for exactly this
 * reason. Runtime text goes through `SMText.t` at the point it is written.
 *
 * `data-i18n` on an element replaces its text. `data-i18n-attr` takes a
 * comma-separated `attribute:KEY` list for titles, placeholders and ARIA
 * labels, which are just as visible and easier to forget.
 */
(function () {
  "use strict";

  const LANGUAGES = [
    { code: "en", label: "English" },
    { code: "ru", label: "Русский" }
  ];
  const FALLBACK = "en";
  const STORAGE_KEY = "scrapmap.language";

  const dictionaries = new Map();
  let active = FALLBACK;

  function translate(key, fallbackText) {
    const table = dictionaries.get(active);
    if (table && typeof table[key] === "string") return table[key];
    const base = dictionaries.get(FALLBACK);
    if (base && typeof base[key] === "string") return base[key];
    // The literal already in the markup is a better last resort than the key.
    return typeof fallbackText === "string" ? fallbackText : key;
  }

  function applyTo(root) {
    const scope = root || document;
    scope.querySelectorAll("[data-i18n]").forEach((element) => {
      const key = element.getAttribute("data-i18n");
      const text = translate(key, element.textContent);
      if (element.textContent !== text) element.textContent = text;
    });
    scope.querySelectorAll("[data-i18n-attr]").forEach((element) => {
      element
        .getAttribute("data-i18n-attr")
        .split(",")
        .forEach((pair) => {
          const [attribute, key] = pair.split(":").map((part) => part.trim());
          if (!attribute || !key) return;
          element.setAttribute(attribute, translate(key, element.getAttribute(attribute)));
        });
    });
    const title = translate("APP_TITLE", null);
    if (title) document.title = title;
  }

  /**
   * Loads every dictionary at once.
   *
   * The native host serves them from the binary, because fetching them over
   * the asset protocol failed silently and left the panel showing raw keys.
   * A browser -- which is how this page is tested -- still fetches.
   */
  let loading = null;
  function loadAll() {
    if (loading) return loading;
    loading = (async () => {
      const invoke = window.__TAURI__?.core?.invoke;
      if (typeof invoke === "function") {
        try {
          const tables = await invoke("interface_strings");
          Object.entries(tables || {}).forEach(([code, table]) => {
            if (table && typeof table === "object") dictionaries.set(code, table);
          });
          if (dictionaries.size) return;
        } catch (_error) {
          /* fall through to fetch */
        }
      }
      await Promise.all(
        LANGUAGES.map(async (entry) => {
          try {
            const response = await fetch(`locales/${entry.code}.json`);
            if (!response.ok) return;
            const table = await response.json();
            if (table && typeof table === "object") dictionaries.set(entry.code, table);
          } catch (_error) {
            /* a language that will not load falls back to English */
          }
        })
      );
    })();
    return loading;
  }

  async function preferred() {
    // Under the overlay the tray owns this setting: it is where the user
    // changes it, and it is read before a WebView exists. Asking the host keeps
    // the menu tick and the panel from disagreeing.
    const invoke = window.__TAURI__?.core?.invoke;
    if (typeof invoke === "function") {
      try {
        const hosted = String(await invoke("interface_language"));
        if (LANGUAGES.some((entry) => entry.code === hosted)) return hosted;
      } catch (_error) {
        /* fall through to the browser's own preference */
      }
    }
    try {
      const stored = window.localStorage?.getItem(STORAGE_KEY);
      if (stored && LANGUAGES.some((entry) => entry.code === stored)) return stored;
    } catch (_error) {
      /* private mode, or no storage at all */
    }
    // Offer the user's own language when we happen to have it, English if not.
    const browser = String(window.navigator?.language || "").slice(0, 2).toLowerCase();
    return LANGUAGES.some((entry) => entry.code === browser) ? browser : FALLBACK;
  }

  async function setLanguage(code) {
    const wanted = LANGUAGES.some((entry) => entry.code === code) ? code : FALLBACK;
    await loadAll();
    active = dictionaries.has(wanted) ? wanted : FALLBACK;
    try {
      window.localStorage?.setItem(STORAGE_KEY, active);
    } catch (_error) {
      /* not worth failing a language change over */
    }
    applyTo(document);
    document.documentElement.lang = active;
    window.dispatchEvent(new CustomEvent("sm-minimap:language", { detail: { language: active } }));
    return active;
  }

  window.SMText = {
    languages: LANGUAGES,
    t: translate,
    apply: applyTo,
    current: () => active,
    setLanguage
  };

  // Before anything else paints, so the panel is never briefly in the wrong
  // language. app.js reads strings through SMText.t, which falls back to the
  // markup until the dictionary lands.
  preferred().then(setLanguage);
  // The markup may not have parsed yet on first run.
  window.addEventListener("DOMContentLoaded", () => {
    applyTo(document);
  });
})();
