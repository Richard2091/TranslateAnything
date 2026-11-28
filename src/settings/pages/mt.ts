// 机器翻译：内置传统翻译引擎（启用 / 停用 + 拖动排序 = 调用顺序 + 参数）
import {
  AI_KINDS,
  S,
  applyMethod,
  cfg,
  engineChain,
  engineInfo,
  kindOf,
  methodOf,
  refresh,
  saveConfig,
  setEngineChain,
  testEngine,
} from "../state";
import { t } from "../../i18n";
import { h, icon, pageHead, section, switchEl, toast } from "../ui";
import { paramFields, sortable, statusBanner, testButton, testNote } from "./engineShared";

/** 展开了参数区的引擎（只在本次打开设置窗口期间记住） */
const opened = new Set<string>();

function tags(id: string, kind: string): HTMLElement[] {
  const info = engineInfo(kind);
  const out: HTMLElement[] = [];
  if (info) {
    out.push(h("span", { class: `tag${info.official ? " accent" : ""}` }, info.official ? t("engines.tag.official") : t("engines.tag.free")));
    if (info.needs_key) out.push(h("span", { class: "tag" }, t("engines.tag.needsKey")));
  }
  if (S.cooldowns.includes(id)) {
    out.push(h("span", { class: "tag warn" }, t("engines.tag.cooling")));
  }
  return out;
}

async function enable(id: string, name: string) {
  const ok = await testEngine(id);
  if (!ok) {
    opened.add(id);
    refresh();
    toast(t("engines.toast.testFirst", { name }), true);
    return;
  }
  const n = engineChain(cfg()).filter((v) => v !== id).length + 1;
  await saveConfig((x) => setEngineChain(x, [...engineChain(x).filter((v) => v !== id), id]), t("engines.toast.enabled", { name, n }));
}

function disable(id: string, name: string) {
  return saveConfig((x) => setEngineChain(x, engineChain(x).filter((v) => v !== id)), t("engines.toast.disabled", { name }));
}

function moveBy(id: string, d: number) {
  const ch = engineChain(cfg());
  const i = ch.indexOf(id);
  const j = i + d;
  if (i < 0 || j < 0 || j >= ch.length) return;
  [ch[i], ch[j]] = [ch[j], ch[i]];
  void saveConfig((x) => setEngineChain(x, ch), "");
}

function engineRow(id: string, chain: string[]): HTMLElement {
  const c = cfg();
  const kind = kindOf(c, id);
  const info = engineInfo(kind);
  const name = info?.name ?? id;
  const fields = info?.fields ?? [];
  const pos = chain.indexOf(id);
  const on = pos >= 0;
  const last = on && chain.length === 1;
  const open = opened.has(id) && fields.length > 0;
  const busy = !!S.tests[id]?.busy;

  const grip = h(
    "button",
    {
      class: "engine-grip",
      type: "button",
      disabled: !on,
      title: on ? t("engines.order.drag") : t("engines.order.dragOff"),
      "aria-label": t("engines.order.drag"),
      "data-k": `grip-${id}`,
      onkeydown: (e: KeyboardEvent) => {
        if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          moveBy(id, e.key === "ArrowUp" ? -1 : 1);
        }
      },
    },
    icon("grip"),
  );

  const sw = switchEl(on, (v) => void (v ? enable(id, name) : disable(id, name)), busy || last);
  sw.title = last ? t("engines.order.keepOne") : on ? t("engines.order.off") : t("engines.order.on");

  const row = h(
    "div",
    { class: `engine-row${on ? " on" : ""}${open ? " open" : ""}`, "data-id": id },
    grip,
    h(
      "button",
      {
        class: `engine-pick${fields.length ? "" : " static"}`,
        type: "button",
        "aria-expanded": fields.length ? String(open) : undefined,
        title: fields.length ? t("engines.order.params") : undefined,
        onclick: () => {
          if (!fields.length) return;
          if (opened.has(id)) opened.delete(id);
          else opened.add(id);
          refresh();
        },
      },
      h("span", { class: "engine-num" }, on ? String(pos + 1) : ""),
      h("div", { class: "grow" }, h("div", { class: "engine-name" }, name, ...tags(id, kind)), h("div", { class: "desc" }, info?.note ?? "")),
      fields.length > 0 && h("span", { class: "engine-chev" }, icon("down")),
    ),
    testButton(id),
    sw,
  );
  if (open) row.append(h("div", { class: "engine-params pop-in" }, paramFields(id, fields), testNote(id)));
  else {
    const n = testNote(id, "");
    if (n) row.append(h("div", { class: "engine-params" }, n));
  }
  return row;
}

export function renderMt(root: HTMLElement) {
  const c = cfg();
  const method = methodOf(c);
  const chain = engineChain(c);
  const traditional = S.catalog.filter((e) => !AI_KINDS.has(e.id));

  const banner = statusBanner(
    method === "engine"
      ? {
          active: true,
          title: t("mt.banner.activeTitle"),
          desc: t("mt.banner.activeDesc"),
        }
      : {
          active: false,
          title: t("mt.banner.backupTitle"),
          desc: t("mt.banner.backupDesc"),
          onAction: () => void saveConfig((x) => { applyMethod(x, "engine"); }, t("engines.toast.toEngine")),
        },
  );

  const ids = [...chain, ...traditional.map((e) => e.id).filter((id) => !chain.includes(id))];
  const engineList = h("div", { class: "card flush engine-list" });
  for (const id of ids) engineList.append(engineRow(id, chain));
  sortable(engineList, (order) => saveConfig((x) => setEngineChain(x, order), t("engines.toast.order")));

  root.replaceChildren(
    pageHead(t("mt.lead")),
    banner,
    section(t("mt.section.list"), method === "llm" && h("span", { class: "count" }, t("engines.section.asBackup"))),
    h("div", { class: "desc list-hint" }, method === "llm" ? t("engines.order.descLlm") : t("engines.order.desc")),
    engineList,
  );
}
