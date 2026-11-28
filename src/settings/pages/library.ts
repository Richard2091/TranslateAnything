// 译文库：搜索、改译、删除、清空
import { invoke } from "@tauri-apps/api/core";
import { cfg, loadStats, providerLabel } from "../state";
import { fmtNum, t } from "../../i18n";
import { confirmDialog, emptyState, errText, h, icon, pageHead, relTime, toast } from "../ui";

interface Entry {
  lang: string;
  source: string;
  target: string;
  engine: string;
  manual: boolean;
  hits: number;
  used_at: number;
}

const PAGE_SIZE = 50;
const lib = { keyword: "", manualOnly: false, offset: 0, total: 0, rows: [] as Entry[] };
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
    }, 300);
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
        h("label", { class: "check" }, manual, t("library.manualOnly")),
        h("button", { class: "btn ghost danger", onclick: clearLibrary }, icon("trash"), t("library.clearEllipsis")),
      ),
      h("div", { id: "lib-table" }),
    ),
  );
  built = true;
}

export async function queryLibrary() {
  try {
    const page = await invoke<{ total: number; rows: Entry[] }>("store_query", {
      keyword: lib.keyword,
      manualOnly: lib.manualOnly,
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
    box.replaceChildren(emptyState(lib.keyword || lib.manualOnly ? t("library.noMatch") : t("library.empty"), lib.keyword ? "search" : "box"));
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
        await invoke("store_set", { source: r.source, target: v });
        r.target = v;
        r.manual = true;
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
        h("td", { class: "src-text" }, r.source),
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
                  await invoke("store_delete", { sources: [r.source] });
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
  const ok = await confirmDialog(
    t("library.clear.title"),
    h(
      "div",
      {},
      h("p", { class: "muted", style: "margin-top:0" }, t("library.clear.body", { lang: cfg().target_lang })),
      h("label", { class: "check" }, keep, t("library.clear.keepManual")),
    ),
    t("library.clear.ok"),
    true,
  );
  if (!ok) return;
  try {
    const n = await invoke<number>("store_clear", { keepManual: keep.checked });
    toast(t("library.toast.cleared", { n: fmtNum(n) }));
    lib.offset = 0;
    queryLibrary();
    loadStats();
  } catch (e) {
    toast(errText(e), true);
  }
}
