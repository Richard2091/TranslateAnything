// 译文库：搜索、按软件筛选、改译、删除、清空
import { invoke } from "@tauri-apps/api/core";
import { StoreAppSummary, cfg, loadStats, providerLabel } from "../state";
import { fmtNum, t } from "../../i18n";
import { confirmDialog, emptyState, errText, h, icon, pageHead, relTime, toast } from "../ui";

interface Entry {
  lang: string;
  scope: string;
  app_name: string;
  source: string;
  target: string;
  engine: string;
  manual: boolean;
  keep: boolean;
  hits: number;
  used_at: number;
}

const PAGE_SIZE = 50;
const ALL_APPS = "__all__";

const lib = {
  keyword: "",
  manualOnly: false,
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

  const appSelect = h("select", { id: "lib-app-select", class: "lib-app" });
  appSelect.addEventListener("change", () => {
    lib.app = appSelect.value === ALL_APPS ? null : appSelect.value;
    lib.offset = 0;
    queryLibrary();
  });

  const manual = h("input", { type: "checkbox", checked: lib.manualOnly });
  manual.addEventListener("change", () => {
    lib.manualOnly = manual.checked;
    lib.offset = 0;
    queryLibrary();
  });

  root.replaceChildren(
    pageHead(t("library.lead")),
    h(
      "div",
      { class: "card flush" },
      h(
        "div",
        { class: "toolbar" },
        h("div", { class: "grow search" }, icon("search"), search),
        appSelect,
        h("label", { class: "check" }, manual, t("library.manualOnly")),
        h("button", { class: "btn ghost danger", onclick: clearLibrary }, icon("trash"), t("library.clearEllipsis")),
      ),
      h("div", { id: "lib-table" }),
    ),
  );
  renderAppSelect();
  built = true;
}

function appLabel(a: StoreAppSummary): string {
  const base = a.id === "" ? t("library.appUnclassified") : a.name || a.id;
  return `${base} (${fmtNum(a.count)})`;
}

function renderAppSelect() {
  const select = document.getElementById("lib-app-select") as HTMLSelectElement | null;
  if (!select) return;
  const opts = [
    h("option", { value: ALL_APPS }, t("library.appAll")),
    ...lib.apps.map((a) => h("option", { value: a.id }, appLabel(a))),
  ];
  select.replaceChildren(...opts);
  select.value = lib.app === null ? ALL_APPS : lib.app;
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
      manualOnly: lib.manualOnly,
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

function renderTable() {
  const box = document.getElementById("lib-table");
  if (!box) return;
  if (lib.rows.length === 0) {
    const hasFilter = Boolean(lib.keyword || lib.manualOnly || lib.app !== null);
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

    const tags = h("div", { class: "lib-tags" });
    if (r.scope !== "") tags.append(h("span", { class: "tag" }, r.app_name || r.scope));
    if (r.keep) tags.append(h("span", { class: "tag warn" }, t("library.keepTag")));

    tbody.append(
      h(
        "tr",
        { class: r.manual ? "manual" : "" },
        h("td", { class: "src-text" }, r.source, tags.childNodes.length > 0 && tags),
        h("td", {}, input),
        h("td", {}, h("span", { class: `tag${r.manual ? " accent" : ""}` }, r.manual ? t("library.manual") : providerLabel(cfg(), r.engine))),
        h("td", { class: "small muted num" }, fmtNum(r.hits)),
        h("td", { class: "small muted" }, relTime(r.used_at)),
        h(
          "td",
          {},
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
          h("th", { style: "width:36%" }, t("library.col.source")),
          h("th", {}, t("library.col.target")),
          h("th", { style: "width:92px" }, t("library.col.engine")),
          h("th", { style: "width:60px" }, t("library.col.hits")),
          h("th", { style: "width:92px" }, t("library.col.used")),
          h("th", { style: "width:52px" }, ""),
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
