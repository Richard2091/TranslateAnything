// AI 翻译：多供应商管理（OpenAI 兼容 / Anthropic 原生 / Gemini 原生）、多启 Fallback 与排序
import {
  S,
  aiChain,
  aiIds,
  applyMethod,
  cfg,
  kindOf,
  methodOf,
  providerLabel,
  refresh,
  saveConfig,
  setAiChain,
  testEngine,
} from "../state";
import { t } from "../../i18n";
import { confirmDialog, dropdown, emptyState, field, h, icon, openDialog, pageHead, section, switchEl, toast } from "../ui";
import {
  AI_PRESETS,
  blankProvider,
  defaultBase,
  hostOf,
  paramFields,
  presetName,
  protocolLabel,
  protocolOptions,
  setParam,
  sortable,
  statusBanner,
  testButton,
  testNote,
  type AiPreset,
} from "./engineShared";

const opened = new Set<string>();

function nextProviderId(c: ReturnType<typeof cfg>, baseSlug: string): string {
  const clean = baseSlug.replace(/[^a-z0-9_]+/gi, "_").toLowerCase() || "ai";
  let i = 1;
  while (c.engines?.[`ai_${clean}_${i}`]) i++;
  return `ai_${clean}_${i}`;
}

function uniqueName(c: ReturnType<typeof cfg>, baseName: string): string {
  const existing = new Set(aiIds(c).map((id) => providerLabel(c, id)));
  if (!existing.has(baseName)) return baseName;
  let i = 2;
  while (existing.has(`${baseName} ${i}`)) i++;
  return `${baseName} ${i}`;
}

async function addFromPreset(preset: AiPreset) {
  const c = cfg();
  const id = nextProviderId(c, preset.id);
  const name = uniqueName(c, presetName(preset));
  const ok = await saveConfig((x) => {
    x.engines[id] = {
      ...blankProvider(),
      name,
      type: preset.kind,
      base_url: preset.baseUrl,
    };
  }, "");
  if (!ok) return;
  opened.add(id);
  refresh();
  requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-k="eng-${id}-api_key"]`)?.focus());
}

async function duplicateProvider(srcId: string) {
  const c = cfg();
  const src = c.engines[srcId];
  if (!src) return;
  const kind = kindOf(c, srcId);
  const id = nextProviderId(c, kind);
  const name = uniqueName(c, providerLabel(c, srcId));
  const ok = await saveConfig((x) => {
    x.engines[id] = { ...src, name };
  }, t("ai.toast.duplicated", { name }));
  if (!ok) return;
  opened.add(id);
  refresh();
}

async function removeProvider(id: string) {
  const c = cfg();
  const name = providerLabel(c, id);
  const ok = await confirmDialog(
    t("ai.remove.title", { name }),
    h("p", { class: "muted" }, t("ai.remove.body")),
    t("common.delete"),
    true,
  );
  if (!ok) return;
  delete S.tests[id];
  opened.delete(id);
  const wasLlm = methodOf(c) === "llm";
  const nextChain = aiChain(c).filter((x) => x !== id);
  const msg = wasLlm && nextChain.length === 0 ? t("ai.toast.removedFallback", { name }) : t("ai.toast.removed", { name });
  await saveConfig((x) => {
    delete x.engines[id];
    setAiChain(x, aiChain(x).filter((v) => v !== id));
  }, msg);
}

async function enable(id: string, name: string) {
  const ok = await testEngine(id);
  if (!ok) {
    opened.add(id);
    refresh();
    toast(t("engines.toast.testFirst", { name }), true);
    return;
  }
  const n = aiChain(cfg()).filter((v) => v !== id).length + 1;
  await saveConfig((x) => setAiChain(x, [...aiChain(x).filter((v) => v !== id), id]), t("engines.toast.enabled", { name, n }));
}

function disable(id: string, name: string) {
  const c = cfg();
  const wasLlm = methodOf(c) === "llm";
  const next = aiChain(c).filter((v) => v !== id);
  const msg = wasLlm && next.length === 0 ? t("ai.toast.disabledFallback", { name }) : t("engines.toast.disabled", { name });
  return saveConfig((x) => setAiChain(x, aiChain(x).filter((v) => v !== id)), msg);
}

function moveBy(id: string, d: number) {
  const ch = aiChain(cfg());
  const i = ch.indexOf(id);
  const j = i + d;
  if (i < 0 || j < 0 || j >= ch.length) return;
  [ch[i], ch[j]] = [ch[j], ch[i]];
  void saveConfig((x) => setAiChain(x, ch), "");
}

function openAddDialog() {
  const groups: { title: string; items: AiPreset[] }[] = [
    { title: t("ai.add.native"), items: AI_PRESETS.filter((p) => p.group === "native") },
    { title: t("ai.add.openai"), items: AI_PRESETS.filter((p) => p.group === "openai" || p.group === "custom") },
  ];
  const dlg = openDialog(
    t("ai.add.title"),
    h(
      "div",
      {},
      h("p", { class: "muted", style: "margin-top:0" }, t("ai.add.desc")),
      ...groups.map((g) =>
        h(
          "div",
          { class: "preset-group" },
          h("div", { class: "preset-group-title" }, g.title),
          h(
            "div",
            { class: "preset-grid" },
            ...g.items.map((p) =>
              h(
                "button",
                {
                  type: "button",
                  class: "preset-item",
                  onclick: () => {
                    dlg.close();
                    void addFromPreset(p);
                  },
                },
                h("div", { class: "preset-name" }, presetName(p)),
                h("div", { class: "preset-sub" }, p.hint || protocolLabel(p.kind)),
              ),
            ),
          ),
        ),
      ),
    ),
    [h("button", { class: "btn", onclick: () => dlg.close() }, t("common.cancel"))],
    { wide: true },
  );
}

function aiRow(id: string, chain: string[]): HTMLElement {
  const c = cfg();
  const p = c.engines[id] ?? blankProvider();
  const kind = kindOf(c, id);
  const name = providerLabel(c, id);
  const pos = chain.indexOf(id);
  const on = pos >= 0;
  const open = opened.has(id);
  const busy = !!S.tests[id]?.busy;
  const host = hostOf(p.base_url || defaultBase(kind, p));
  const model = p.model?.trim() || t("dash.noModel");
  const summary = host ? `${model} · ${host}` : model;

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

  const sw = switchEl(on, (v) => void (v ? enable(id, name) : disable(id, name)), busy);
  sw.title = on ? t("engines.order.off") : t("engines.order.on");

  const copyBtn = h(
    "button",
    { type: "button", class: "icon-btn", title: t("ai.duplicate"), "aria-label": t("ai.duplicate"), onclick: () => void duplicateProvider(id) },
    icon("copy"),
  );
  const delBtn = h(
    "button",
    { type: "button", class: "icon-btn danger", title: t("common.delete"), "aria-label": t("common.delete"), onclick: () => void removeProvider(id) },
    icon("trash"),
  );

  const row = h(
    "div",
    { class: `engine-row${on ? " on" : ""}${open ? " open" : ""}`, "data-id": id },
    grip,
    h(
      "button",
      {
        class: "engine-pick",
        type: "button",
        "aria-expanded": String(open),
        title: t("engines.order.params"),
        onclick: () => {
          if (opened.has(id)) opened.delete(id);
          else opened.add(id);
          refresh();
        },
      },
      h("span", { class: "engine-num" }, on ? String(pos + 1) : ""),
      h(
        "div",
        { class: "grow" },
        h(
          "div",
          { class: "engine-name" },
          name,
          h("span", { class: "tag" }, protocolLabel(kind)),
          S.cooldowns.includes(id) && h("span", { class: "tag warn" }, t("engines.tag.cooling")),
        ),
        h("div", { class: "desc clip" }, summary),
      ),
      h("span", { class: "engine-chev" }, icon("down")),
    ),
    testButton(id),
    copyBtn,
    delBtn,
    sw,
  );

  if (open) {
    const nameInput = h("input", {
      type: "text",
      value: p.name?.trim() || name,
      placeholder: t("engines.field.name.ph"),
      "data-k": `eng-${id}-name`,
      autocomplete: "off",
      spellcheck: false,
    });
    nameInput.addEventListener("change", () => void setParam(id, "name", nameInput.value));

    const protoDrop = dropdown(
      protocolOptions(),
      kind,
      (nextKind) => {
        delete S.tests[id];
        void saveConfig((x) => {
          const cur = (x.engines[id] ??= blankProvider());
          const oldDef = defaultBase(kind, cur);
          cur.type = nextKind;
          if (!cur.base_url.trim() || cur.base_url.trim() === oldDef) {
            cur.base_url = "";
          }
        });
      },
      { width: "100%" },
    );

    const extra = [field(t("engines.field.name.label"), nameInput), field(t("engines.field.protocol.label"), protoDrop)];
    row.append(h("div", { class: "engine-params pop-in" }, paramFields(id, ["base_url", "api_key", "model", "proxy"], true, extra), testNote(id)));
  } else {
    const n = testNote(id, "");
    if (n) row.append(h("div", { class: "engine-params" }, n));
  }
  return row;
}

export function renderAi(root: HTMLElement) {
  const c = cfg();
  const method = methodOf(c);
  const allIds = aiIds(c);
  const chain = aiChain(c);

  const banner = statusBanner(
    method === "llm"
      ? {
          active: true,
          title: t("ai.banner.activeTitle"),
          desc: t("ai.banner.activeDesc"),
        }
      : {
          active: false,
          title: t("ai.banner.standbyTitle"),
          desc: chain.length ? t("ai.banner.standbyDesc", { n: chain.length }) : t("ai.banner.needProvider"),
          actionDisabled: chain.length === 0,
          actionTitle: chain.length === 0 ? t("ai.banner.needProvider") : undefined,
          onAction: () => void saveConfig((x) => { applyMethod(x, "llm"); }, t("engines.toast.toLlm")),
        },
  );

  const addBtn = h("button", { type: "button", class: "btn primary sm", onclick: openAddDialog }, icon("plus"), t("ai.addBtn"));

  let listEl: HTMLElement;
  if (!allIds.length) {
    const quick = AI_PRESETS.filter((p) => ["deepseek", "openai", "anthropic", "gemini", "siliconflow", "ollama", "custom"].includes(p.id));
    listEl = h(
      "div",
      { class: "card" },
      emptyState(t("ai.empty"), "sparkle"),
      h("div", { class: "desc", style: "text-align:center;margin:-4px 0 14px" }, t("ai.emptyHint")),
      h(
        "div",
        { class: "preset-quick" },
        ...quick.map((p) =>
          h("button", { type: "button", class: "btn sm", onclick: () => void addFromPreset(p) }, icon("plus"), presetName(p)),
        ),
      ),
    );
  } else {
    const ids = [...chain, ...allIds.filter((id) => !chain.includes(id))];
    listEl = h("div", { class: "card flush engine-list" });
    for (const id of ids) listEl.append(aiRow(id, chain));
    sortable(listEl, (order) => saveConfig((x) => setAiChain(x, order), t("engines.toast.order")));
  }

  const parts: HTMLElement[] = [
    pageHead(t("ai.lead")),
    banner,
    section(
      t("ai.section.providers"),
      allIds.length > 0 && h("span", { class: "count" }, t("ai.count", { enabled: chain.length, total: allIds.length })),
      h("span", { class: "spacer" }),
      addBtn,
    ),
  ];
  if (allIds.length > 0) parts.push(h("div", { class: "desc list-hint" }, t("ai.order.desc")));
  parts.push(listEl);
  root.replaceChildren(...parts);
}
