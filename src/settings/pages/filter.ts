// 过滤规则与最近过滤诊断：全局与按应用规则编辑、实时过滤日志、一键纠错
import { invoke } from "@tauri-apps/api/core";
import { FilterEvent, FilterRuleSet, cfg, loadStats, loadView, saveConfig } from "../state";
import { t } from "../../i18n";
import { emptyState, errText, h, icon, pageHead, toast } from "../ui";
import { GLOBAL_SCOPE, Scope, collectScopes as collectAllScopes, loadScopeApps, normalizeAppId, scopeDropdown as scopePicker } from "./appScope";

interface Draft {
  keepPatterns: string;
  forceTranslate: string;
}

let selectedScope = GLOBAL_SCOPE;
let draft: Draft | null = null;
/** 规则编辑框有未保存修改时，外部 config-changed 不覆盖正在编辑的内容 */
let dirty = false;
let onlyKept = true;
let events: FilterEvent[] = [];

export const filterDirty = () => dirty;
export function resetFilterDraft() {
  if (!dirty) draft = null;
}

function dedup(items: string[]): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const item of items) {
    const v = item.trim();
    if (!v || seen.has(v)) continue;
    seen.add(v);
    out.push(v);
  }
  return out;
}

function parseLines(raw: string): string[] {
  return dedup(raw.split(/\r?\n/));
}

function collectScopes(): Scope[] {
  return collectAllScopes(Object.keys(cfg().filter?.apps ?? {}));
}

function readDraft(scope: string): Draft {
  const c = cfg();
  if (scope === GLOBAL_SCOPE) {
    return {
      keepPatterns: (c.filter?.keep_patterns ?? []).join("\n"),
      forceTranslate: (c.filter?.force_translate ?? []).join("\n"),
    };
  }
  const keepPatterns: string[] = [];
  const forceTranslate: string[] = [];
  const pushSet = (rs?: FilterRuleSet) => {
    if (!rs) return;
    keepPatterns.push(...(rs.keep_patterns ?? []));
    forceTranslate.push(...(rs.force_translate ?? []));
  };
  for (const [k, rs] of Object.entries(c.filter?.apps ?? {})) {
    if (normalizeAppId(k) === scope) pushSet(rs);
  }
  for (const tg of c.immersive.targets) {
    const exeId = normalizeAppId(tg.exe);
    const nameId = normalizeAppId(tg.name);
    if ((exeId && exeId === scope) || (nameId && nameId === scope)) {
      pushSet(tg.filter);
    }
  }
  return {
    keepPatterns: dedup(keepPatterns).join("\n"),
    forceTranslate: dedup(forceTranslate).join("\n"),
  };
}

function scopeDropdown(scopes: Scope[], onPick: () => void): HTMLElement {
  return scopePicker(scopes, selectedScope, (v) => {
    selectedScope = v;
    dirty = false;
    draft = null;
    onPick();
  });
}

/** 软件列表加载完后刷新作用范围下拉的选项 */
function populateScopeSelect() {
  const slot = document.getElementById("filter-scope-slot");
  if (!slot) return;
  const scopes = collectScopes();
  if (!scopes.some((s) => s.id === selectedScope)) {
    selectedScope = GLOBAL_SCOPE;
    dirty = false;
    draft = null;
  }
  const host = document.getElementById("tab-filter");
  slot.replaceChildren(
    scopeDropdown(scopes, () => {
      if (host) renderFilter(host);
      void loadFilterEvents();
    }),
  );
}

export function renderFilter(root: HTMLElement) {
  const scopes = collectScopes();
  if (!scopes.some((s) => s.id === selectedScope)) {
    selectedScope = GLOBAL_SCOPE;
    dirty = false;
    draft = null;
  }
  if (!draft) draft = readDraft(selectedScope);
  const current = draft;
  const rerender = () => renderFilter(root);

  const scopeSelect = h(
    "span",
    { id: "filter-scope-slot" },
    scopeDropdown(scopes, () => {
      rerender();
      void loadFilterEvents();
    }),
  );

  const keepPatternsEl = h("textarea", { class: "rule-box", placeholder: t("filter.keepPatterns.ph") }, current.keepPatterns);
  keepPatternsEl.addEventListener("input", () => {
    current.keepPatterns = keepPatternsEl.value;
    dirty = true;
  });

  const forceEl = h("textarea", { class: "rule-box", placeholder: t("filter.forceTranslate.ph") }, current.forceTranslate);
  forceEl.addEventListener("input", () => {
    current.forceTranslate = forceEl.value;
    dirty = true;
  });

  const save = async () => {
    const nextRule: FilterRuleSet = {
      keep_patterns: parseLines(current.keepPatterns),
      force_translate: parseLines(current.forceTranslate),
    };
    const empty = nextRule.keep_patterns.length === 0 && nextRule.force_translate.length === 0;
    const savedScope = selectedScope;
    const prevDraft = { ...current };

    dirty = false;
    draft = null;
    const ok = await saveConfig((c) => {
      c.filter ??= { keep_patterns: [], force_translate: [], apps: {} };
      if (savedScope === GLOBAL_SCOPE) {
        c.filter.keep_patterns = nextRule.keep_patterns;
        c.filter.force_translate = nextRule.force_translate;
      } else {
        c.filter.apps ??= {};
        for (const key of Object.keys(c.filter.apps)) {
          if (normalizeAppId(key) === savedScope && key !== savedScope) {
            delete c.filter.apps[key];
          }
        }
        if (empty) delete c.filter.apps[savedScope];
        else c.filter.apps[savedScope] = nextRule;

        for (const tg of c.immersive.targets) {
          const exeId = normalizeAppId(tg.exe);
          const nameId = normalizeAppId(tg.name);
          if ((exeId && exeId === savedScope) || (nameId && nameId === savedScope)) {
            tg.filter = { keep_patterns: [], force_translate: [] };
          }
        }
      }
    }, t("filter.toast.saved"));

    if (!ok) {
      dirty = true;
      draft = prevDraft;
      rerender();
    } else {
      await loadFilterEvents();
    }
  };

  const onlyKeptCb = h("input", { type: "checkbox", checked: onlyKept });
  onlyKeptCb.addEventListener("change", () => {
    onlyKept = onlyKeptCb.checked;
    void loadFilterEvents();
  });

  root.replaceChildren(
    pageHead(t("filter.lead")),
    h(
      "div",
      { class: "card" },
      h(
        "div",
        { class: "row spread", style: "gap:12px; margin-bottom:14px; flex-wrap:wrap" },
        h("div", { class: "grow" }, h("h3", {}, t("filter.rules.title")), h("div", { class: "desc" }, t("filter.rules.desc"))),
        scopeSelect,
        h("button", { class: "btn primary", onclick: save }, t("common.save")),
      ),
      h(
        "div",
        { class: "filter-rules-grid" },
        h("label", { class: "field" }, h("span", {}, t("filter.keepPatterns.label")), keepPatternsEl),
        h("label", { class: "field" }, h("span", {}, t("filter.forceTranslate.label")), forceEl),
      ),
    ),
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "toolbar" },
        h("div", { class: "grow" }, h("h3", {}, t("filter.log.title")), h("div", { class: "desc" }, t("filter.log.desc"))),
        h("label", { class: "check" }, onlyKeptCb, t("filter.log.onlyKept")),
        h("button", { class: "btn ghost sm", onclick: () => void loadFilterPage() }, icon("refresh"), t("filter.log.refresh")),
      ),
      h("div", { id: "filter-log" }),
    ),
  );

  renderLog();
}

function renderLog() {
  const box = document.getElementById("filter-log");
  if (!box) return;
  if (events.length === 0) {
    box.replaceChildren(emptyState(t("filter.log.empty")));
    return;
  }

  const tbody = h("tbody");
  for (const ev of events) {
    const isKeep = ev.decision === "keep";
    const action = isKeep ? "translate" : "keep";
    const btnLabel = isKeep ? t("filter.action.toTranslate") : t("filter.action.toKeep");
    const appText = ev.app_name || ev.app_id || t("library.appUnclassified");

    tbody.append(
      h(
        "tr",
        {},
        h("td", { class: "src-text" }, ev.text),
        h(
          "td",
          {},
          h("div", { class: "row", style: "gap:6px; flex-wrap:wrap" }, h("span", { class: "small" }, appText), h("span", { class: "tag" }, t(`filter.mode.${ev.mode}`))),
        ),
        h("td", {}, h("span", { class: `tag${isKeep ? " warn" : ""}` }, t(`filter.reason.${ev.reason}`))),
        h(
          "td",
          {},
          h(
            "button",
            {
              class: "btn ghost sm",
              onclick: async () => {
                try {
                  await invoke("filter_override", {
                    text: ev.text,
                    action,
                    appId: ev.app_id || null,
                  });
                  toast(t(isKeep ? "filter.toast.toTranslate" : "filter.toast.toKeep"));
                  await loadView();
                  resetFilterDraft();
                  if (!dirty) {
                    const host = document.getElementById("tab-filter");
                    if (host) renderFilter(host);
                  }
                  await loadFilterPage();
                  loadStats();
                } catch (e) {
                  toast(errText(e), true);
                }
              },
            },
            btnLabel,
          ),
        ),
      ),
    );
  }

  box.replaceChildren(
    h(
      "table",
      {},
      h(
        "thead",
        {},
        h(
          "tr",
          {},
          h("th", { style: "width:42%" }, t("filter.col.text")),
          h("th", { style: "width:22%" }, t("filter.col.app")),
          h("th", { style: "width:18%" }, t("filter.col.reason")),
          h("th", { style: "width:110px" }, t("filter.col.action")),
        ),
      ),
      tbody,
    ),
  );
}

async function loadFilterEvents() {
  try {
    events = await invoke<FilterEvent[]>("filter_events", {
      limit: 100,
      appId: selectedScope === GLOBAL_SCOPE ? null : selectedScope,
      onlyKept,
    });
    renderLog();
  } catch (e) {
    toast(errText(e), true);
  }
}

export async function loadFilterPage() {
  try {
    const [, evs] = await Promise.all([
      loadScopeApps(),
      invoke<FilterEvent[]>("filter_events", {
        limit: 100,
        appId: selectedScope === GLOBAL_SCOPE ? null : selectedScope,
        onlyKept,
      }),
    ]);
    events = evs ?? [];
    populateScopeSelect();
    renderLog();
  } catch (e) {
    toast(errText(e), true);
  }
}
