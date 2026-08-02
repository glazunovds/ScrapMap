/**
 * Interface strings, keyed and looked up at runtime.
 *
 * English is the default and the fallback. A language is a file in `locales/`
 * plus an entry in `LANGUAGES` below, so adding one needs no code changes
 * elsewhere; a key missing from a translation falls back to English rather than
 * rendering blank, which keeps a half-finished translation usable.
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

  async function load(code) {
    if (dictionaries.has(code)) return true;
    try {
      const response = await fetch(`locales/${code}.json`, { cache: "no-cache" });
      if (!response.ok) return false;
      const table = await response.json();
      if (!table || typeof table !== "object") return false;
      dictionaries.set(code, table);
      return true;
    } catch (_error) {
      return false;
    }
  }

  function preferred() {
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
    await load(FALLBACK);
    if (wanted !== FALLBACK) await load(wanted);
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
  setLanguage(preferred());
  // The markup may not have parsed yet on first run.
  window.addEventListener("DOMContentLoaded", () => {
    applyTo(document);
  });
})();
