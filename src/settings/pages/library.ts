// 译文库：搜索、按软件筛选、改译、删除、清空
import { invoke } from "@tauri-apps/api/core";
import { StoreAppSummary, cfg, loadStats, providerLabel } from "../state";
import { fmtNum, t } from "../../i18n";
import { collapse, collapsible, confirmDialog, emptyState, errText, expand, h, icon, pageHead, searchSelect, segmented, toast, type DropOption } from "../ui";

interface AppRef {
  id: string;
  name: string;
}

interface Entry {
  lang: string;
  scope: string;
  app_name: string;
  source: string;
  target: string;
  engine: string;
  manual: boolean;
  keep: boolean;
  /** 归属的软件：按应用隔离的是所在软件，共享长句是用过它的所有软件 */
  apps: AppRef[];
}

const PAGE_SIZE = 50;
const ALL_APPS = "__all__";

type SourceFilter = "all" | "manual" | "auto";
type KeepFilter = "all" | "only" | "exclude";

const lib = {
  keyword: "",
  /** 更多筛选：来源、保留原文 */
  source: "all" as SourceFilter,
  keep: "all" as KeepFilter,
  app: null as string | null,
  apps: [] as StoreAppSummary[],
  offset: 0,
  total: 0,
  rows: [] as Entry[],
};
let built = false;
let searchTimer = 0;

/** 译文库页只搭一次骨架（保住搜索框里正在输入的内容），之后只刷新表格 */
export function buildLibrary(root: HTMLElement) {
  if (built) return;
  const search = h("input", { type: "search", placeholder: t("library.search"), value: lib.keyword });
  search.addEventListener("input", () => {
    clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => {
      lib.keyword = search.value;
      lib.offset = 0;
      queryLibrary();
    }, 180);
  });

  // 更多筛选：按钮在工具栏里，面板在工具栏下方，用统一的展开 / 收起效果
  const filterSlot = h("div", { class: "lib-filter-slot" });
  const filterBtn = h(
    "button",
    {
      type: "button",
      class: "btn ghost lib-filter-btn",
      "aria-expanded": "false",
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
    h("span", { class: "filter-count", id: "lib-filter-count" }),
    icon("down"),
  );

  root.replaceChildren(
    pageHead(t("library.lead")),
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "toolbar" },
        h("div", { class: "grow search" }, icon("search"), search),
        h("span", { id: "lib-app-slot" }),
        filterBtn,
        h("button", { class: "btn ghost danger", onclick: clearLibrary }, icon("trash"), t("library.clearEllipsis")),
      ),
      filterSlot,
      h("div", { id: "lib-table" }),
    ),
  );
  syncFilterBtn();
  renderAppSelect();
  built = true;
}

let filtersOpen = false;

/** 生效中的更多筛选条件数，显示在按钮上 */
function syncFilterBtn() {
  const n = Number(lib.source !== "all") + Number(lib.keep !== "all");
  const badge = document.getElementById("lib-filter-count");
  if (!badge) return;
  badge.textContent = n ? String(n) : "";
  badge.hidden = !n;
}

function filterPanel(): HTMLElement {
  const apply = () => {
    lib.offset = 0;
    syncFilterBtn();
    queryLibrary();
  };
  const row = (label: string, control: HTMLElement) => h("div", { class: "lib-filter-row" }, h("span", { class: "lib-filter-label" }, label), control);
  const panel = h("div", { class: "lib-filters" });
  const fill = () =>
    panel.replaceChildren(
      row(
        t("library.filter.source"),
        segmented<SourceFilter>(
          [
            ["all", t("library.filter.all")],
            ["manual", t("library.filter.manual")],
            ["auto", t("library.filter.auto")],
          ],
          lib.source,
          (v) => ((lib.source = v), apply()),
        ),
      ),
      row(
        t("library.filter.keep"),
        segmented<KeepFilter>(
          [
            ["all", t("library.filter.all")],
            ["only", t("library.filter.keepOnly")],
            ["exclude", t("library.filter.keepExclude")],
          ],
          lib.keep,
          (v) => ((lib.keep = v), apply()),
        ),
      ),
      h(
        "button",
        {
          type: "button",
          class: "link lib-filter-reset",
          onclick: () => {
            if (lib.source === "all" && lib.keep === "all") return;
            lib.source = "all";
            lib.keep = "all";
            fill();
            apply();
          },
        },
        t("library.filter.reset"),
      ),
    );
  fill();
  return panel;
}

function appLabel(a: StoreAppSummary): string {
  const base = a.id === "" ? t("library.appUnclassified") : a.name || a.id;
  return `${base} (${fmtNum(a.count)})`;
}

/** 上次渲染的软件下拉内容；没变就不重建，免得每次查询都换掉按钮 */
let appSelectSig = "";

/** 沉浸模式里配置过的程序路径，按程序名（小写）索引；译文库只存程序名，路径从这里补来参与搜索 */
function exePaths(): Map<string, string> {
  const m = new Map<string, string>();
  for (const tg of cfg().immersive.targets) {
    const exe = tg.exe.trim();
    const base = (exe.split(/[\\/]/).pop() ?? "").toLowerCase();
    if (base && exe !== base) m.set(base, exe);
  }
  return m;
}

function renderAppSelect() {
  const slot = document.getElementById("lib-app-slot");
  if (!slot) return;
  const paths = exePaths();
  const options: DropOption<string>[] = [
    ...lib.apps.map((a) => ({
      value: a.id,
      label: appLabel(a),
      // 显示名和程序名不同时把程序名作为小字提示
      hint: a.id && a.name && a.name.toLowerCase() !== a.id.toLowerCase() ? a.id : undefined,
      keywords: paths.get(a.id.toLowerCase()),
    })),
  ];
  const value = lib.app === null ? ALL_APPS : lib.app;
  const sig = JSON.stringify([value, options]);
  // 没变化，或正在里面输入时不重建（免得打断）
  if ((sig === appSelectSig && slot.firstChild) || slot.contains(document.activeElement)) return;
  appSelectSig = sig;
  slot.replaceChildren(
    searchSelect(
      options,
      value,
      (v) => {
        lib.app = v === ALL_APPS ? null : v;
        lib.offset = 0;
        queryLibrary();
      },
      { placeholder: t("library.appSearch"), width: "220px", empty: ALL_APPS },
    ),
  );
}

export async function queryLibrary() {
  try {
    const apps = await invoke<StoreAppSummary[]>("store_apps");
    lib.apps = apps ?? [];
    if (lib.app !== null && !lib.apps.some((a) => a.id === lib.app)) {
      lib.app = null;
      lib.offset = 0;
    }
    renderAppSelect();

    const page = await invoke<{ total: number; rows: Entry[] }>("store_query", {
      keyword: lib.keyword,
      manual: lib.source === "all" ? null : lib.source === "manual",
      keep: lib.keep === "all" ? null : lib.keep === "only",
      app: lib.app,
      offset: lib.offset,
      limit: PAGE_SIZE,
    });
    lib.total = page.total;
    lib.rows = page.rows;
    renderTable();
  } catch (e) {
    toast(errText(e), true);
  }
}

/** 软件名的首字（跳过空白和括号里的补充说明） */
function initial(name: string): string {
  const s = name.replace(/[（(].*$/, "").trim() || name.trim();
  return [...s][0]?.toUpperCase() ?? "?";
}

/** 按程序名算一个稳定的色相，同一软件每次颜色一样 */
function hueOf(id: string): number {
  let n = 0;
  for (const ch of id) n = (n * 31 + ch.charCodeAt(0)) >>> 0;
  return n % 360;
}

const MAX_APPS = 3;

/**
 * “软件”列：每个软件一个首字图标 + 名称；列窄时只显示图标（见 CSS 容器查询）。
 * 共享长句可能被多个软件用过，最多列 3 个，其余折成 +N。点一下按这个软件筛选。
 */
function appCell(apps: AppRef[]): HTMLElement {
  if (!apps.length) return h("span", { class: "lib-app-none", title: t("library.appUnclassified") }, "—");
  const chip = (a: AppRef) => {
    const name = a.name || a.id;
    return h(
      "button",
      {
        type: "button",
        class: "lib-app-chip",
        title: name === a.id ? name : `${name}\n${a.id}`,
        onclick: () => {
          lib.app = a.id;
          lib.offset = 0;
          queryLibrary();
        },
      },
      h("span", { class: "app-avatar", style: `--h:${hueOf(a.id)}` }, initial(name)),
      h("span", { class: "lib-app-name" }, name),
    );
  };
  const rest = apps.length - MAX_APPS;
  return h(
    "div",
    { class: "lib-apps" },
    ...apps.slice(0, rest > 0 ? MAX_APPS - 1 : MAX_APPS).map(chip),
    rest > 0 && h("span", { class: "lib-app-more", title: apps.slice(MAX_APPS - 1).map((a) => a.name || a.id).join("\n") }, `+${rest + 1}`),
  );
}

function renderTable() {
  const box = document.getElementById("lib-table");
  if (!box) return;
  if (lib.rows.length === 0) {
    const hasFilter = Boolean(lib.keyword || lib.app !== null || lib.source !== "all" || lib.keep !== "all");
    box.replaceChildren(emptyState(hasFilter ? t("library.noMatch") : t("library.empty"), lib.keyword ? "search" : "box"));
    return;
  }
  const tbody = h("tbody");
  for (const r of lib.rows) {
    const input = h("textarea", { class: "cell", rows: 1, title: t("library.editHint") });
    input.value = r.target;
    const commit = async () => {
      const v = input.value.trim();
      if (!v || v === r.target) {
        input.value = r.target;
        return;
      }
      try {
        await invoke("store_set", { source: r.source, target: v, scope: r.scope });
        r.target = v;
        r.manual = true;
        r.keep = v === r.source.trim();
        toast(t("library.toast.edited"));
        renderTable();
        loadStats();
      } catch (e) {
        toast(errText(e), true);
      }
    };
    input.addEventListener("change", commit);
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        input.blur();
      }
    });

    tbody.append(
      h(
        "tr",
        { class: r.manual ? "manual" : "" },
        h("td", {}, appCell(r.apps)),
        h("td", { class: "src-text" }, r.source, r.keep && h("div", { class: "lib-tags" }, h("span", { class: "tag warn" }, t("library.keepTag")))),
        h("td", {}, input),
        (() => {
          const label = r.manual ? t("library.manual") : providerLabel(cfg(), r.engine);
          return h("td", {}, h("span", { class: `tag lib-engine${r.manual ? " accent" : ""}`, title: label }, label));
        })(),
        h(
          "td",
          { class: "lib-del-col" },
          h(
            "button",
            {
              class: "icon-btn",
              title: t("library.deleteHint"),
              "aria-label": t("common.delete"),
              onclick: async () => {
                try {
                  await invoke("store_delete", { sources: [r.source], scopes: [r.scope] });
                  toast(t("library.toast.deleted"));
                  queryLibrary();
                  loadStats();
                } catch (e) {
                  toast(errText(e), true);
                }
              },
            },
            icon("trash"),
          ),
        ),
      ),
    );
  }
  const pages = Math.max(1, Math.ceil(lib.total / PAGE_SIZE));
  const cur = Math.floor(lib.offset / PAGE_SIZE) + 1;
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
          h("th", { class: "lib-app-col" }, t("library.col.app")),
          h("th", { style: "width:34%" }, t("library.col.source")),
          h("th", {}, t("library.col.target")),
          h("th", { class: "lib-engine-col" }, t("library.col.engine")),
          h("th", { class: "lib-del-col" }, ""),
        ),
      ),
      tbody,
    ),
    h(
      "div",
      { class: "row spread pager" },
      h("span", { class: "muted small" }, t("library.total", { n: fmtNum(lib.total), lang: cfg().target_lang })),
      h(
        "div",
        { class: "row" },
        h("button", { class: "btn sm", disabled: cur <= 1, onclick: () => ((lib.offset -= PAGE_SIZE), queryLibrary()) }, icon("chevL"), t("library.prev")),
        h("span", { class: "small muted num" }, `${cur} / ${pages}`),
        h("button", { class: "btn sm", disabled: cur >= pages, onclick: () => ((lib.offset += PAGE_SIZE), queryLibrary()) }, t("library.next"), icon("chevR")),
      ),
    ),
  );
}

async function clearLibrary() {
  const keep = h("input", { type: "checkbox", checked: true });
  const lang = cfg().target_lang;
  let bodyText: string;
  if (lib.app === null) {
    bodyText = t("library.clear.body", { lang });
  } else if (lib.app === "") {
    bodyText = t("library.clear.bodyUnclassified", { lang });
  } else {
    const appName = lib.apps.find((a) => a.id === lib.app)?.name || lib.app;
    bodyText = t("library.clear.bodyApp", { app: appName, lang });
  }

  const ok = await confirmDialog(
    t("library.clear.title"),
    h(
      "div",
      {},
      h("p", { class: "muted", style: "margin-top:0" }, bodyText),
      h("label", { class: "check" }, keep, t("library.clear.keepManual")),
    ),
    t("library.clear.ok"),
    true,
  );
  if (!ok) return;
  try {
    const n = await invoke<number>("store_clear", { keepManual: keep.checked, app: lib.app });
    toast(t("library.toast.cleared", { n: fmtNum(n) }));
    lib.offset = 0;
    queryLibrary();
    loadStats();
  } catch (e) {
    toast(errText(e), true);
  }
}
