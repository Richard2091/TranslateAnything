// AI 翻译：多供应商管理（OpenAI 兼容 / Anthropic 原生 / Gemini 原生）、多启 Fallback 与排序
import {
  AI_KINDS,
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
import {
  confirmDialog,
  dropdown,
  emptyState,
  field,
  h,
  icon,
  openDialog,
  pageHead,
  section,
  switchEl,
  toast,
} from "../ui";
import {
  AI_PRESETS,
  blankProvider,
  defaultBase,
  hostOf,
  paramFields,
  presetMatches,
  presetName,
  presetOf,
  protocolLabel,
  protocolOptions,
  setParam,
  sortable,
  statusBanner,
  testButton,
  wireParams,
  type AiPreset,
} from "./engineShared";
import { hasLogo, pickImage, providerIcon, providerLogo, pruneSiteIcons } from "../logos";

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

/** 按预设新增；自定义接口新增一条空白配置（不选协议、不填名称和地址），由用户自己填 */
function addFromPreset(preset: AiPreset) {
  if (preset.group === "custom") return addProvider("custom", "", "", "");
  return addProvider(preset.id, presetName(preset), preset.kind, preset.baseUrl);
}

async function addProvider(slug: string, baseName: string, kind: string, baseUrl: string) {
  const c = cfg();
  const id = nextProviderId(c, slug);
  const name = baseName && uniqueName(c, baseName);
  const ok = await saveConfig((x) => {
    x.engines[id] = {
      ...blankProvider(),
      name,
      type: kind,
      base_url: baseUrl,
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
  const ok2 = await saveConfig((x) => {
    delete x.engines[id];
    setAiChain(x, aiChain(x).filter((v) => v !== id));
  }, msg);
  // 上传的图片随配置一起删了；网站图标缓存按域名存，没有别的引擎用同一域名就删掉
  if (ok2) pruneSiteIcons();
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
    { title: t("ai.add.vendor"), items: AI_PRESETS.filter((p) => p.group === "vendor") },
    { title: t("ai.add.platform"), items: AI_PRESETS.filter((p) => p.group === "platform") },
    { title: t("ai.add.local"), items: AI_PRESETS.filter((p) => p.group === "local") },
    { title: t("ai.add.custom"), items: AI_PRESETS.filter((p) => p.group === "custom") },
  ];
  const pick = (p: AiPreset) => {
    dlg.close();
    void addFromPreset(p);
  };
  // 每个预设一个按钮，搜索时只切换显隐，不重建
  const items = groups.map((g) => ({
    group: g,
    buttons: g.items.map((p) => ({
      preset: p,
      el: h(
        "button",
        { type: "button", class: "preset-item", onclick: () => pick(p) },
        providerLogo(p.group === "custom" ? undefined : p.id),
        h(
          "div",
          { class: "preset-text" },
          h("div", { class: "preset-name" }, presetName(p)),
          h("div", { class: "preset-sub" }, p.group === "custom" ? t("ai.custom.hint") : p.hint || protocolLabel(p.kind)),
        ),
      ),
    })),
  }));
  const groupEls = items.map(({ group, buttons }) =>
    h("div", { class: "preset-group" }, h("div", { class: "preset-group-title" }, group.title), h("div", { class: "preset-grid" }, ...buttons.map((b) => b.el))),
  );
  const empty = h("p", { class: "muted preset-empty" }, t("ai.add.noMatch"));
  empty.hidden = true;
  const search = h("input", { type: "search", placeholder: t("ai.add.search") });
  const visible = () => items.flatMap((g) => g.buttons).filter((b) => !b.el.hidden);
  const filter = () => {
    const q = search.value;
    let hits = 0;
    for (const { buttons } of items)
      for (const b of buttons) {
        b.el.hidden = !presetMatches(b.preset, q);
        if (!b.el.hidden) hits++;
      }
    // 一个都没命中时留下"自定义"，总有路可走
    const custom = items.flatMap((g) => g.buttons).find((b) => b.preset.group === "custom");
    if (hits === 0 && custom) custom.el.hidden = false;
    items.forEach((g, i) => (groupEls[i].hidden = g.buttons.every((b) => b.el.hidden)));
    empty.hidden = hits > 0;
  };
  search.addEventListener("input", filter);
  search.addEventListener("keydown", (e) => {
    if (e.key !== "Enter") return;
    const first = visible()[0];
    if (first) pick(first.preset);
  });
  const list = h("div", { class: "preset-list" }, empty, ...groupEls);
  const body = h(
    "div",
    {},
    h("div", { class: "preset-search" }, h("p", { class: "muted" }, t("ai.add.desc")), h("div", { class: "search" }, icon("search"), search)),
    list,
  );
  const actions = [h("button", { class: "btn", onclick: () => dlg.close() }, t("common.cancel"))];
  const dlg = openDialog(t("ai.add.title"), body, actions, { wide: true });
  // 固定列表区高度，筛选时对话框不跟着跳
  requestAnimationFrame(() => (list.style.minHeight = `${list.offsetHeight}px`));
}

/** 供应商当前显示的图标；自动模式按接口地址（留空时按协议默认地址）匹配内置 logo */
function rowIcon(id: string, override?: string): HTMLElement {
  const c = cfg();
  const p = { ...(c.engines[id] ?? blankProvider()) };
  if (override !== undefined) p.icon = override;
  const kind = kindOf(c, id);
  return providerIcon(id, p, presetOf(kind, p.base_url || defaultBase(kind, p))?.id);
}

function setIcon(id: string, value: string) {
  return saveConfig((x) => {
    const p = (x.engines[id] ??= blankProvider());
    if (value) p.icon = value;
    else delete p.icon;
  }, "");
}

/** 选图标：自动 / 通用 / 上传的图片 / 内置 logo */
function openIconPicker(id: string) {
  const cur = cfg().engines[id]?.icon?.trim() ?? "";
  const pick = (value: string) => {
    dlg.close();
    void setIcon(id, value);
  };
  const tile = (value: string, logo: HTMLElement, label: string, onclick = () => pick(value)) =>
    h("button", { type: "button", class: `icon-tile${value === cur ? " selected" : ""}`, title: label, onclick }, logo, h("span", { class: "clip" }, label));
  const upload = async () => {
    const data = await pickImage();
    if (data === null) return;
    if (!data) return toast(t("ai.icon.badImage"), true);
    pick(data);
  };
  const special = [
    tile("", rowIcon(id, ""), t("ai.icon.auto")),
    tile("none", providerLogo(undefined), t("ai.icon.none")),
    cur.startsWith("data:image/") && tile(cur, rowIcon(id), t("ai.icon.custom")),
    tile("upload", h("span", { class: "logo generic", "aria-hidden": "true" }, icon("plus")), t("ai.icon.upload"), () => void upload()),
  ];
  // 只显示主名称，去掉括号里的补充说明（如“阿里云百炼（通义千问）”→“阿里云百炼”）
  const shortName = (p: AiPreset) => presetName(p).replace(/\s*[（(][^（）()]*[）)]\s*/g, " ").trim();
  const builtin = AI_PRESETS.filter((p) => hasLogo(p.id)).map((p) => tile(`preset:${p.id}`, providerLogo(p.id), shortName(p)));
  const body = h(
    "div",
    {},
    h("p", { class: "muted" }, t("ai.icon.desc")),
    h("div", { class: "icon-grid" }, ...special),
    h("div", { class: "preset-group-title" }, t("ai.icon.builtin")),
    h("div", { class: "icon-grid" }, ...builtin),
  );
  const dlg = openDialog(t("ai.icon.title"), body, [h("button", { class: "btn", onclick: () => dlg.close() }, t("common.cancel"))]);
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

  const pick = h(
    "button",
    {
      class: "engine-pick",
      type: "button",
      "aria-expanded": String(open),
      title: t("engines.order.params"),
    },
      h("span", { class: "engine-num" }, on ? String(pos + 1) : ""),
      rowIcon(id),
      h(
        "div",
        { class: "grow" },
        h(
          "div",
          { class: "engine-name" },
          name,
          h("span", { class: `tag${AI_KINDS.has(kind) ? "" : " warn"}` }, protocolLabel(kind)),
          S.cooldowns.includes(id) && h("span", { class: "tag warn" }, t("engines.tag.cooling")),
        ),
        h("div", { class: "desc clip" }, summary),
      ),
    h("span", { class: "engine-chev" }, icon("down")),
  );

  const row = h("div", { class: `engine-row${on ? " on" : ""}${open ? " open" : ""}`, "data-id": id }, grip, pick, testButton(id), copyBtn, delBtn, sw);

  const buildParams = () => {
    const nameInput = h("input", {
      type: "text",
      value: p.name?.trim() ?? "",
      placeholder: p.name?.trim() ? t("engines.field.name.ph") : name,
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
      { width: "100%", placeholder: t("ai.custom.pickProtocol") },
    );

    const iconBtn = h(
      "button",
      { type: "button", class: "icon-pick", title: t("ai.icon.change"), "aria-label": t("ai.icon.change"), onclick: () => openIconPicker(id) },
      rowIcon(id),
    );
    // 图标和名称并排，各带自己的标题
    const nameBox = h("div", { class: "name-with-icon" }, field(t("ai.icon.label"), iconBtn), field(t("engines.field.name.label"), nameInput));
    const extra = [nameBox, field(t("engines.field.protocol.label"), protoDrop)];
    return h("div", { class: "engine-params" }, paramFields(id, ["base_url", "api_key", "model", "concurrency", "proxy"], true, extra));
  };

  wireParams(row, pick, id, opened, buildParams);
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

  const addBtn = h("button", { type: "button", class: "btn primary sm", onclick: () => openAddDialog() }, icon("plus"), t("ai.addBtn"));

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
          h(
            "button",
            { type: "button", class: "btn sm", onclick: () => void addFromPreset(p) },
            icon("plus"),
            presetName(p),
          ),
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
