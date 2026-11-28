// 术语表：词怎么翻——自定义术语（指定译法 / 保留原文，全局或按软件）、内置术语表开关与查看。
// 筛选方式与译文库一致：先选软件，再搜原文 / 译文，"更多筛选"里按类型筛
import { S, cfg, saveConfig } from "../state";
import { fmtNum, t } from "../../i18n";
import { collapse, collapsible, emptyState, errText, expand, h, icon, pageHead, searchSelect, segmented, switchEl, toast, type DropOption } from "../ui";
import { GLOBAL_SCOPE, Scope, collectScopes, loadScopeApps, normalizeAppId } from "./appScope";

interface Row {
  src: string;
  dst: string;
  /** 保留原文：保存时译文写成原文 */
  keep: boolean;
}

type TypeFilter = "all" | "translate" | "keep";

let selectedScope = GLOBAL_SCOPE;
let draft: Row[] | null = null;
/** 自定义术语有未保存的修改（此时配置变动不覆盖正在编辑的内容） */
let dirty = false;
let keyword = "";
let typeFilter: TypeFilter = "all";
let filtersOpen = false;
let builtinFilter = "";

export const glossaryDirty = () => dirty;
export function resetGlossaryDraft() {
  if (!dirty) draft = null;
}

function scopes(): Scope[] {
  return collectScopes(Object.keys(cfg().glossary_apps ?? {}));
}

/** 某个软件已有的自定义术语条数 */
function termCount(scope: string): number {
  return Object.entries(cfg().glossary_apps ?? {})
    .filter(([k]) => normalizeAppId(k) === scope)
    .reduce((n, [, m]) => n + Object.keys(m).length, 0);
}

function readDraft(scope: string): Row[] {
  const c = cfg();
  const maps =
    scope === GLOBAL_SCOPE
      ? [c.glossary]
      : Object.entries(c.glossary_apps ?? {})
          .filter(([k]) => normalizeAppId(k) === scope)
          .map(([, m]) => m);
  const out = new Map<string, string>();
  for (const m of maps) for (const [k, v] of Object.entries(m)) out.set(k, v);
  return [...out].map(([src, dst]) => ({ src, dst, keep: dst.trim() === src.trim() }));
}

/** 切到术语表页时载入译文库里的软件，补全软件选择框 */
export async function loadGlossaryPage() {
  try {
    await loadScopeApps();
  } catch (e) {
    toast(errText(e), true);
    return;
  }
  renderScopeSelect();
}

/** 软件选择框：留空 = 全局术语，可输入搜索软件（同译文库） */
function renderScopeSelect() {
  const slot = document.getElementById("glossary-scope-slot");
  if (!slot || slot.contains(document.activeElement)) return;
  const options: DropOption<string>[] = scopes()
    .filter((s) => s.id !== GLOBAL_SCOPE)
    .map((s) => {
      const n = termCount(s.id);
      return { value: s.id, label: n ? `${s.label} (${fmtNum(n)})` : s.label, hint: s.label !== s.id ? s.id : undefined };
    });
  slot.replaceChildren(
    searchSelect(
      options,
      selectedScope,
      (v) => {
        if (v === selectedScope) return;
        selectedScope = v;
        dirty = false;
        draft = null;
        const host = document.getElementById("tab-glossary");
        if (host) renderGlossary(host);
      },
      { placeholder: t("glossary.scopeSearch"), width: "100%", empty: GLOBAL_SCOPE },
    ),
  );
}

/** 当前搜索词与类型下是否显示这一行（还没填原文的新行总是显示） */
function visible(r: Row): boolean {
  if (!r.src.trim()) return true;
  if (typeFilter === "keep" && !r.keep) return false;
  if (typeFilter === "translate" && r.keep) return false;
  const k = keyword.trim().toLowerCase();
  return !k || r.src.toLowerCase().includes(k) || (!r.keep && r.dst.toLowerCase().includes(k));
}

export function renderGlossary(root: HTMLElement) {
  if (!scopes().some((s) => s.id === selectedScope)) {
    selectedScope = GLOBAL_SCOPE;
    dirty = false;
    draft = null;
  }
  if (!draft) draft = readDraft(selectedScope);
  const rows = draft;
  const rerender = () => renderGlossary(root);

  const tbody = h("tbody");
  /** 只重画表格（搜索、筛选时用，保住搜索框焦点） */
  const fillRows = () => {
    const trs = rows.flatMap((row, i) => {
      if (!visible(row)) return [];
      const src = h("input", { type: "text", value: row.src, placeholder: t("glossary.sourcePh"), "data-k": `gl-s-${i}` });
      const dst = h("input", {
        type: "text",
        value: row.keep ? "" : row.dst,
        placeholder: row.keep ? t("glossary.keepPh") : t("glossary.targetPh"),
        disabled: row.keep,
        "data-k": `gl-t-${i}`,
      });
      const keep = h("input", { type: "checkbox", checked: row.keep, "aria-label": t("glossary.col.keep") });
      src.addEventListener("input", () => ((row.src = src.value), (dirty = true)));
      dst.addEventListener("input", () => ((row.dst = dst.value), (dirty = true)));
      keep.addEventListener("change", () => {
        row.keep = keep.checked;
        if (!row.keep && row.dst.trim() === row.src.trim()) row.dst = "";
        dirty = true;
        fillRows();
      });
      return [
        h(
          "tr",
          {},
          h("td", {}, src),
          h("td", {}, dst),
          h("td", { style: "text-align:center" }, keep),
          h(
            "td",
            {},
            h("button", { class: "icon-btn", title: t("common.delete"), "aria-label": t("common.delete"), onclick: () => (rows.splice(i, 1), (dirty = true), rerender()) }, icon("trash")),
          ),
        ),
      ];
    });
    if (trs.length === 0) {
      const filtered = rows.length > 0;
      trs.push(h("tr", {}, h("td", { colspan: 4 }, emptyState(filtered ? t("glossary.noMatch") : t("glossary.empty"), filtered ? "search" : "box"))));
    }
    tbody.replaceChildren(...trs);
  };
  fillRows();

  const save = async () => {
    const map: Record<string, string> = {};
    for (const r of rows) {
      const k = r.src.trim();
      if (!k) continue;
      if (!r.keep && !r.dst.trim()) {
        toast(t("glossary.toast.emptyTarget", { term: k }), true);
        return;
      }
      map[k] = r.keep ? k : r.dst.trim();
    }
    const scope = selectedScope;
    dirty = false;
    draft = null;
    const ok = await saveConfig((c) => {
      if (scope === GLOBAL_SCOPE) {
        c.glossary = map;
        return;
      }
      const apps = (c.glossary_apps ??= {});
      for (const key of Object.keys(apps)) if (normalizeAppId(key) === scope) delete apps[key];
      if (Object.keys(map).length > 0) apps[scope] = map;
    }, t("glossary.toast.saved"));
    if (!ok) {
      dirty = true;
      draft = rows;
      rerender();
    }
  };

  const search = h("input", { type: "search", placeholder: t("library.search"), value: keyword });
  search.addEventListener("input", () => {
    keyword = search.value;
    fillRows();
  });

  // 更多筛选：与译文库相同的按钮 + 展开面板
  const filterSlot = h("div", { class: "lib-filter-slot" });
  const badge = h("span", { class: "filter-count" });
  const syncBadge = () => {
    badge.textContent = typeFilter === "all" ? "" : "1";
    badge.hidden = typeFilter === "all";
  };
  syncBadge();
  const filterPanel = () => {
    const panel = h("div", { class: "lib-filters" });
    const fill = () =>
      panel.replaceChildren(
        h(
          "div",
          { class: "lib-filter-row" },
          h("span", { class: "lib-filter-label" }, t("glossary.filter.type")),
          segmented<TypeFilter>(
            [
              ["all", t("library.filter.all")],
              ["translate", t("glossary.filter.translate")],
              ["keep", t("glossary.col.keep")],
            ],
            typeFilter,
            (v) => ((typeFilter = v), syncBadge(), fillRows()),
          ),
        ),
        h(
          "button",
          {
            type: "button",
            class: "link lib-filter-reset",
            onclick: () => {
              if (typeFilter === "all") return;
              typeFilter = "all";
              syncBadge();
              fill();
              fillRows();
            },
          },
          t("library.filter.reset"),
        ),
      );
    fill();
    return panel;
  };
  const filterBtn = h(
    "button",
    {
      type: "button",
      class: `btn ghost lib-filter-btn${filtersOpen ? " open" : ""}`,
      "aria-expanded": String(filtersOpen),
      onclick: () => {
        filtersOpen = !filtersOpen;
        filterBtn.classList.toggle("open", filtersOpen);
        filterBtn.setAttribute("aria-expanded", String(filtersOpen));
        const cur = filterSlot.querySelector<HTMLElement>(":scope > .collapse");
        if (!filtersOpen) {
          if (cur) collapse(cur, () => cur.remove());
          return;
        }
        if (cur) return expand(cur);
        const wrap = collapsible(filterPanel());
        filterSlot.append(wrap);
        expand(wrap);
      },
    },
    icon("sliders"),
    t("library.moreFilters"),
    badge,
    icon("down"),
  );
  // 重画整页时保持面板展开，不重播动画
  if (filtersOpen) filterSlot.append(collapsible(filterPanel()));

  const builtin = Object.entries(S.view.builtin_glossary);
  const filterInput = h("input", { type: "search", placeholder: t("glossary.filter"), value: builtinFilter });
  const builtinBody = h("tbody");
  const fillBuiltin = () => {
    const f = builtinFilter.toLowerCase();
    builtinBody.replaceChildren(
      ...builtin
        .filter(([k, v]) => !f || k.toLowerCase().includes(f) || v.includes(builtinFilter))
        .map(([k, v]) => h("tr", {}, h("td", {}, k), h("td", {}, v))),
    );
  };
  filterInput.addEventListener("input", () => {
    builtinFilter = filterInput.value;
    fillBuiltin();
  });
  fillBuiltin();

  root.replaceChildren(
    pageHead(t("glossary.lead")),
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "toolbar" },
        h("div", { class: "grow" }, h("h3", {}, t("glossary.custom.title")), h("div", { class: "desc" }, t("glossary.custom.desc", { n: fmtNum(rows.length) }))),
        h("button", { class: "btn ghost", onclick: () => (rows.push({ src: "", dst: "", keep: false }), (dirty = true), rerender()) }, icon("plus"), t("glossary.addRow")),
        h("button", { class: "btn primary", onclick: save }, t("common.save")),
      ),
      h("div", { class: "toolbar" }, h("span", { id: "glossary-scope-slot" }), h("div", { class: "grow search lib-search" }, icon("search"), search), filterBtn),
      filterSlot,
      h(
        "table",
        {},
        h(
          "thead",
          {},
          h(
            "tr",
            {},
            h("th", {}, t("glossary.col.source")),
            h("th", {}, t("glossary.col.target")),
            h("th", { style: "width:88px; text-align:center" }, t("glossary.col.keep")),
            h("th", { style: "width:52px" }, ""),
          ),
        ),
        tbody,
      ),
    ),
    h(
      "div",
      { class: "card" },
      h(
        "div",
        { class: "row spread", style: "gap:16px" },
        h(
          "div",
          { class: "grow" },
          h("h3", {}, t("glossary.builtin.title")),
          h("div", { class: "desc" }, t("glossary.builtin.desc", { n: fmtNum(builtin.length), lang: cfg().target_lang })),
        ),
        switchEl(cfg().builtin_glossary, (v) => saveConfig((c) => (c.builtin_glossary = v), v ? t("glossary.toast.builtinOn") : t("glossary.toast.builtinOff"))),
      ),
      builtin.length > 0 &&
        h(
          "details",
          { class: "pages", style: "margin-top:14px" },
          h("summary", {}, t("glossary.builtin.view")),
          h("div", { class: "search", style: "margin:10px 0 8px" }, icon("search"), filterInput),
          h("div", { class: "inner-table" }, h("table", {}, h("thead", {}, h("tr", {}, h("th", {}, t("glossary.col.source")), h("th", {}, t("glossary.col.target")))), builtinBody)),
        ),
    ),
  );
  renderScopeSelect();
}
