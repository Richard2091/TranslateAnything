// 机器翻译与 AI 翻译页面共用的表单控件、测试按钮、拖动排序与状态横幅
import { invoke } from "@tauri-apps/api/core";
import { S, cfg, engineInfo, kindOf, refresh, saveConfig, saving, settled, testEngine, type ProviderConfig } from "../state";
import { t } from "../../i18n";
import { collapse, collapsible, combo, dropdown, expand, field, fmtSec, h, icon, type DropOption } from "../ui";

/** 可配置的参数；标签和提示文字取 engines.field.<参数>.label / .ph */
const FIELDS = new Set(["base_url", "api_key", "region", "model", "concurrency", "proxy"]);
const SECRET = new Set(["api_key"]);
const LLM_PLACEHOLDER = new Set(["base_url"]);

export const blankProvider = (): ProviderConfig => ({ name: "", base_url: "", api_key: "", region: "", model: "", proxy: null, concurrency: null });

/** DeepL 免费版地址（免费版密钥以 :fx 结尾） */
const DEEPL_FREE = "https://api-free.deepl.com";

export function setParam(id: string, key: string, raw: string) {
  const v = raw.trim();
  return saveConfig((c) => {
    const p = (c.engines[id] ??= blankProvider());
    if (key === "proxy") p.proxy = v || null;
    else if (key === "concurrency") {
      const n = Number(v);
      p.concurrency = v && Number.isFinite(n) && n >= 1 ? Math.min(16, Math.round(n)) : null;
    } else (p as unknown as Record<string, string>)[key] = v;
  });
}

export function hostOf(url: string): string {
  const s = url.trim();
  if (!s) return "";
  try {
    return new URL(s).host;
  } catch {
    return s;
  }
}

export function protocolLabel(kind: string): string {
  if (kind === "anthropic") return t("engines.protocol.anthropic");
  if (kind === "gemini") return t("engines.protocol.gemini");
  return t("engines.protocol.openai");
}

export function protocolOptions(): DropOption<string>[] {
  return [
    { value: "openai", label: t("engines.protocol.openai"), hint: "/chat/completions" },
    { value: "anthropic", label: t("engines.protocol.anthropic"), hint: "/v1/messages" },
    { value: "gemini", label: t("engines.protocol.gemini"), hint: "/v1beta/models/*:generateContent" },
  ];
}

export interface AiPreset {
  id: string;
  name?: string;
  nameKey?: string;
  kind: "openai" | "anthropic" | "gemini";
  baseUrl: string;
  hint: string;
  group: "native" | "openai" | "custom";
}

export const AI_PRESETS: AiPreset[] = [
  { id: "anthropic", name: "Anthropic Claude", kind: "anthropic", baseUrl: "", hint: "https://api.anthropic.com", group: "native" },
  { id: "gemini", name: "Google Gemini", kind: "gemini", baseUrl: "", hint: "https://generativelanguage.googleapis.com", group: "native" },
  { id: "openai", name: "OpenAI", kind: "openai", baseUrl: "https://api.openai.com/v1", hint: "https://api.openai.com/v1", group: "openai" },
  { id: "deepseek", name: "DeepSeek", kind: "openai", baseUrl: "https://api.deepseek.com/v1", hint: "https://api.deepseek.com/v1", group: "openai" },
  { id: "dashscope", nameKey: "engines.preset.dashscope", kind: "openai", baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", hint: "dashscope.aliyuncs.com", group: "openai" },
  { id: "volcengine", nameKey: "engines.preset.volcengine", kind: "openai", baseUrl: "https://ark.cn-beijing.volces.com/api/v3", hint: "ark.cn-beijing.volces.com", group: "openai" },
  { id: "zhipu", nameKey: "engines.preset.zhipu", kind: "openai", baseUrl: "https://open.bigmodel.cn/api/paas/v4", hint: "open.bigmodel.cn", group: "openai" },
  { id: "moonshot", nameKey: "engines.preset.moonshot", kind: "openai", baseUrl: "https://api.moonshot.cn/v1", hint: "api.moonshot.cn", group: "openai" },
  { id: "siliconflow", nameKey: "engines.preset.siliconflow", kind: "openai", baseUrl: "https://api.siliconflow.cn/v1", hint: "api.siliconflow.cn", group: "openai" },
  { id: "openrouter", name: "OpenRouter", kind: "openai", baseUrl: "https://openrouter.ai/api/v1", hint: "openrouter.ai", group: "openai" },
  { id: "groq", name: "Groq", kind: "openai", baseUrl: "https://api.groq.com/openai/v1", hint: "api.groq.com", group: "openai" },
  { id: "mistral", name: "Mistral", kind: "openai", baseUrl: "https://api.mistral.ai/v1", hint: "api.mistral.ai", group: "openai" },
  { id: "xai", name: "xAI Grok", kind: "openai", baseUrl: "https://api.x.ai/v1", hint: "api.x.ai", group: "openai" },
  { id: "ollama", nameKey: "engines.preset.ollama", kind: "openai", baseUrl: "http://localhost:11434/v1", hint: "http://localhost:11434/v1", group: "openai" },
  { id: "lmstudio", nameKey: "engines.preset.lmstudio", kind: "openai", baseUrl: "http://localhost:1234/v1", hint: "http://localhost:1234/v1", group: "openai" },
  { id: "custom", nameKey: "engines.preset.custom", kind: "openai", baseUrl: "", hint: "", group: "custom" },
];

export const presetName = (p: AiPreset): string => p.name ?? (p.nameKey ? t(p.nameKey) : p.id);

// ───────────────────────── 候选项 ─────────────────────────

const AZURE_REGIONS: [string, string][] = [
  ["eastasia", "East Asia (Hong Kong)"],
  ["southeastasia", "Southeast Asia (Singapore)"],
  ["japaneast", "Japan East"],
  ["japanwest", "Japan West"],
  ["koreacentral", "Korea Central"],
  ["centralindia", "Central India"],
  ["australiaeast", "Australia East"],
  ["eastus", "East US"],
  ["eastus2", "East US 2"],
  ["westus", "West US"],
  ["westus2", "West US 2"],
  ["westus3", "West US 3"],
  ["centralus", "Central US"],
  ["northcentralus", "North Central US"],
  ["southcentralus", "South Central US"],
  ["westcentralus", "West Central US"],
  ["canadacentral", "Canada Central"],
  ["brazilsouth", "Brazil South"],
  ["northeurope", "North Europe"],
  ["westeurope", "West Europe"],
  ["uksouth", "UK South"],
  ["francecentral", "France Central"],
  ["germanywestcentral", "Germany West Central"],
  ["switzerlandnorth", "Switzerland North"],
  ["norwayeast", "Norway East"],
  ["swedencentral", "Sweden Central"],
  ["uaenorth", "UAE North"],
  ["qatarcentral", "Qatar Central"],
  ["southafricanorth", "South Africa North"],
  ["chinaeast2", "China East 2 (21Vianet)"],
  ["chinanorth3", "China North 3 (21Vianet)"],
];

function regionOptions(): DropOption<string>[] {
  return [
    { value: "global", label: t("engines.region.global"), hint: "global" },
    ...AZURE_REGIONS.map(([value, label]) => ({ value, label, hint: value })),
  ];
}

const BASE_PRESETS: Record<string, [string, string][]> = {
  openai: [
    ["https://api.openai.com/v1", "OpenAI"],
    ["https://api.deepseek.com/v1", "DeepSeek"],
    ["https://dashscope.aliyuncs.com/compatible-mode/v1", "@dashscope"],
    ["https://ark.cn-beijing.volces.com/api/v3", "@volcengine"],
    ["https://open.bigmodel.cn/api/paas/v4", "@zhipu"],
    ["https://api.moonshot.cn/v1", "@moonshot"],
    ["https://api.siliconflow.cn/v1", "@siliconflow"],
    ["https://openrouter.ai/api/v1", "OpenRouter"],
    ["https://api.groq.com/openai/v1", "Groq"],
    ["https://api.mistral.ai/v1", "Mistral"],
    ["https://api.x.ai/v1", "xAI Grok"],
    ["http://localhost:11434/v1", "@ollama"],
    ["http://localhost:1234/v1", "@lmstudio"],
  ],
  anthropic: [["https://api.anthropic.com", "Anthropic Official"]],
  gemini: [["https://generativelanguage.googleapis.com", "Google Gemini Official"]],
  microsoft: [
    ["https://api.cognitive.microsofttranslator.com", "@msGlobal"],
    ["https://api-apc.cognitive.microsofttranslator.com", "@msApc"],
    ["https://api-eur.cognitive.microsofttranslator.com", "@msEur"],
    ["https://api-nam.cognitive.microsofttranslator.com", "@msNam"],
    ["https://api.translator.azure.cn", "@msChina"],
  ],
  deepl: [
    [DEEPL_FREE, "@deeplFree"],
    ["https://api.deepl.com", "@deeplPro"],
  ],
  deeplx: [["http://127.0.0.1:1188/translate", "@deeplxLocal"]],
};

function presetOptions(kind: string): DropOption<string>[] | null {
  const list = BASE_PRESETS[kind];
  if (!list) return null;
  return list.map(([value, label]) => ({
    value,
    label: label.startsWith("@") ? t(`engines.preset.${label.slice(1)}`) : label,
    hint: value,
  }));
}

export function defaultBase(kind: string, p: ProviderConfig): string {
  if (kind === "deepl" && p.api_key.trim().endsWith(":fx")) return DEEPL_FREE;
  return engineInfo(kind)?.default_base_url ?? "";
}

const modelCache = new Map<string, string[]>();

/** 大模型的模型输入框：点右侧按钮从接口获取可用模型，也可以直接填写 */
export function modelCombo(id: string, onCommit: (v: string) => void, key = `eng-${id}-model`): HTMLElement {
  const cacheKey = () => {
    const p = cfg().engines[id] ?? blankProvider();
    return `${kindOf(cfg(), id)}\n${p.base_url}\n${p.api_key}\n${p.proxy ?? ""}\n${cfg().proxy ?? ""}`;
  };
  const toOptions = (ids: string[]) => ids.map((m) => ({ value: m, label: m }));
  const cachedOptions = () => {
    if (saving()) return undefined;
    const ids = modelCache.get(cacheKey());
    return ids && toOptions(ids);
  };
  return combo(cfg().engines[id]?.model ?? "", onCommit, {
    key,
    placeholder: t("engines.field.model.ph"),
    title: t("engines.model.fetch"),
    options: cachedOptions(),
    cached: cachedOptions,
    load: async () => {
      await settled();
      const c = cfg();
      const kind = kindOf(c, id);
      if (kind === "openai" && !c.engines[id]?.base_url?.trim()) {
        throw new Error(t("engines.model.needBase"));
      }
      const hit = modelCache.get(cacheKey());
      if (hit) return toOptions(hit);
      const ids = await invoke<string[]>("llm_models", { id });
      modelCache.set(cacheKey(), ids);
      return toOptions(ids);
    },
  });
}

type ProxyMode = "" | "direct" | "system" | "custom";
const PROXY_SUGGEST: DropOption<string>[] = [
  { value: "http://127.0.0.1:7890", label: "http://127.0.0.1:7890", hint: "Clash / Mihomo" },
  { value: "http://127.0.0.1:7897", label: "http://127.0.0.1:7897", hint: "Clash Verge Rev" },
  { value: "http://127.0.0.1:10809", label: "http://127.0.0.1:10809", hint: "v2rayN" },
  { value: "http://127.0.0.1:1080", label: "http://127.0.0.1:1080", hint: "Shadowsocks" },
];
const customProxy = new Set<string>();

export function proxyField(value: string, scope: string | null, onCommit: (v: string) => void): HTMLElement {
  const engine = scope !== null;
  const tag = scope ?? "";
  const v = value.trim();
  let mode: ProxyMode = v === "system" ? "system" : v === "direct" ? (engine ? "direct" : "") : v ? "custom" : "";
  if (!v && customProxy.has(tag)) mode = "custom";
  const modes: DropOption<ProxyMode>[] = [
    engine ? { value: "", label: t("proxy.followGlobal"), hint: t("proxy.followGlobalHint") } : { value: "", label: t("proxy.direct") },
    ...(engine ? [{ value: "direct" as ProxyMode, label: t("proxy.direct") }] : []),
    { value: "system", label: t("proxy.system"), hint: t("proxy.systemHint") },
    { value: "custom", label: t("proxy.custom"), hint: t("proxy.customHint") },
  ];
  const pick = dropdown(
    modes,
    mode,
    (m) => {
      if (m === "custom") {
        customProxy.add(tag);
        refresh();
        return;
      }
      customProxy.delete(tag);
      onCommit(m);
    },
    { width: "100%" },
  );
  if (mode !== "custom") return pick;
  const addr = combo(
    v,
    (a) => {
      if (a) customProxy.delete(tag);
      onCommit(a);
    },
    { key: `proxy-${tag}`, placeholder: t("proxy.customPh"), options: PROXY_SUGGEST },
  );
  return h("div", { class: "proxy-field" }, pick, addr);
}

/** 某个引擎的参数表单；改完一项（失焦、回车或选中候选项）就保存 */
export function paramFields(id: string, fields: string[], llm = false, extraBefore: HTMLElement[] = []): HTMLElement {
  const p = cfg().engines[id] ?? blankProvider();
  const kind = kindOf(cfg(), id);
  const grid = h("div", { class: "form-grid two" }, ...extraBefore);
  for (const key of fields) {
    if (!FIELDS.has(key)) continue;
    const secret = SECRET.has(key);
    const stored = key === "proxy" ? p.proxy ?? "" : ((p as unknown as Record<string, string>)[key] ?? "");
    const def = key === "base_url" ? defaultBase(kind, p) : "";
    const value = stored || def;
    const label = t(`engines.field.${key}.label`);
    const wide = key === "proxy" || key === "base_url" ? "wide" : "";
    const save = (v: string) => void setParam(id, key, def && v.trim() === def ? "" : v);
    if (key === "concurrency") {
      const num = h("input", {
        type: "number",
        min: "1",
        max: "16",
        value: p.concurrency != null ? String(p.concurrency) : "",
        placeholder: t("engines.field.concurrency.ph"),
        "data-k": `eng-${id}-concurrency`,
      });
      num.addEventListener("change", () => save(num.value));
      grid.append(field(label, num, wide));
      continue;
    }
    if (key === "model") {
      grid.append(field(label, modelCombo(id, save), wide));
      continue;
    }
    if (key === "proxy") {
      grid.append(field(label, proxyField(value, id, save), wide));
      continue;
    }
    const ph = llm && LLM_PLACEHOLDER.has(key) ? t(`engines.field.${key}.llmPh`) : t(`engines.field.${key}.ph`);
    if (key === "region") {
      grid.append(field(label, combo(value, save, { key: `eng-${id}-region`, placeholder: ph, options: regionOptions() }), wide));
      continue;
    }
    const presets = key === "base_url" ? presetOptions(kind) : null;
    if (presets) {
      grid.append(field(label, combo(value, save, { key: `eng-${id}-base_url`, placeholder: ph, options: presets }), wide));
      continue;
    }
    const input = h("input", {
      type: secret ? "password" : "text",
      value,
      placeholder: ph,
      "data-k": `eng-${id}-${key}`,
      autocomplete: "off",
      spellcheck: false,
    });
    input.addEventListener("change", () => save(input.value));
    let control: HTMLElement = input;
    if (secret) {
      const eye = h(
        "button",
        { type: "button", class: "input-icon", title: t("engines.toggleSecret"), onclick: () => (input.type = input.type === "password" ? "text" : "password") },
        icon("eye"),
      );
      control = h("div", { class: "input-wrap" }, input, eye);
    }
    grid.append(field(label, control, wide));
  }
  return grid;
}

/** "测试"按钮：显示耗时（秒，精确到小数点后两位）或失败 */
export function testButton(id: string, cls = "btn sm"): HTMLElement {
  const r = S.tests[id];
  const label = r?.busy
    ? t("engines.test.busy")
    : r?.result
      ? fmtSec(r.result.ms)
      : r?.error
        ? t("engines.test.failed")
        : t("engines.test.label");
  const tone = r?.busy ? "" : r?.result ? (r.result.ms > 2000 ? " slow" : " good") : r?.error ? " fail" : "";
  const title = r?.result ? `${r.result.source}\n→ ${r.result.text}` : r?.error ?? t("engines.test.hint");
  return h("button", { class: `${cls} test-btn${tone}`, disabled: r?.busy, title, onclick: () => void testEngine(id) }, icon("bolt"), label);
}

/** 测试结果的说明行（失败原因或示例译文） */
export function testNote(id: string, style = "margin-top:12px"): HTMLElement | null {
  const r = S.tests[id];
  if (!r || r.busy) return null;
  if (r.error) return h("div", { class: "note bad", style }, r.error);
  if (r.result) return h("div", { class: "note ok", style }, `${r.result.source} → ${r.result.text}`);
  return null;
}

/**
 * 引擎行的参数区：点 `pick` 展开 / 收起，只动这一行并播放过渡（不重绘整页）；
 * 重绘时已展开的直接显示，不重播动画。测试结果放在参数区外面，展开与否位置一致。
 */
export function wireParams(row: HTMLElement, pick: HTMLElement, id: string, opened: Set<string>, build: () => HTMLElement) {
  const n = testNote(id, "");
  const note = n ? h("div", { class: "engine-params" }, n) : null;
  if (opened.has(id)) row.append(collapsible(build()));
  if (note) row.append(note);
  pick.addEventListener("click", () => {
    const isOpen = !opened.has(id);
    if (isOpen) opened.add(id);
    else opened.delete(id);
    row.classList.toggle("open", isOpen);
    pick.setAttribute("aria-expanded", String(isOpen));
    const cur = row.querySelector<HTMLElement>(":scope > .collapse");
    if (!isOpen) {
      if (cur) collapse(cur, () => cur.remove());
      return;
    }
    if (cur) return expand(cur); // 正在收起时又点开
    const wrap = collapsible(build());
    row.insertBefore(wrap, note);
    expand(wrap);
  });
}

/**
 * 按住手柄拖动排序（只在启用的行之间移动）。
 */
export function sortable(list: HTMLElement, onDrop: (ids: string[]) => void) {
  const enabledRows = () => [...list.querySelectorAll<HTMLElement>(".engine-row.on")];
  const order = () => enabledRows().map((r) => r.dataset.id!);

  const flip = (el: HTMLElement, mutate: () => void) => {
    const a = el.offsetTop;
    mutate();
    const b = el.offsetTop;
    el.style.transition = "none";
    el.style.transform = `translateY(${a - b}px)`;
    void el.offsetHeight;
    el.style.transition = "";
    el.style.transform = "";
  };

  list.addEventListener("pointerdown", (e) => {
    const grip = (e.target as Element).closest<HTMLButtonElement>(".engine-grip");
    if (!grip || grip.disabled || e.button !== 0) return;
    const row = grip.closest<HTMLElement>(".engine-row")!;
    e.preventDefault();
    grip.setPointerCapture(e.pointerId);
    const before = order().join();
    const listTop = () => list.getBoundingClientRect().top;
    const offset = e.clientY - listTop() - row.offsetTop;
    row.classList.add("dragging");
    list.classList.add("sorting");

    const place = (clientY: number) => {
      const rows = enabledRows();
      const first = rows[0];
      const lastRow = rows[rows.length - 1];
      let top = clientY - listTop() - offset;
      top = Math.max(first.offsetTop, Math.min(top, lastRow.offsetTop + lastRow.offsetHeight - row.offsetHeight));
      for (let guard = 0; guard < rows.length; guard++) {
        const cur = enabledRows();
        const i = cur.indexOf(row);
        const prev = cur[i - 1];
        const next = cur[i + 1];
        if (prev && top < prev.offsetTop + prev.offsetHeight / 2) flip(prev, () => list.insertBefore(prev, row.nextSibling));
        else if (next && top + row.offsetHeight > next.offsetTop + next.offsetHeight / 2) flip(next, () => list.insertBefore(next, row));
        else break;
      }
      row.style.transform = `translateY(${top - row.offsetTop}px)`;
    };

    const move = (ev: PointerEvent) => place(ev.clientY);
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      row.style.transform = "";
      row.classList.remove("dragging");
      list.classList.remove("sorting");
      const after = order();
      if (after.join() !== before) onDrop(after);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  });
}

/** 子页顶部角色状态横幅：显示当前是否默认，非默认时带「设为默认」按钮 */
export function statusBanner(opts: {
  active: boolean;
  title: string;
  desc: string;
  actionDisabled?: boolean;
  actionTitle?: string;
  onAction?: () => void;
}): HTMLElement {
  return h(
    "div",
    { class: `card status-banner${opts.active ? " active" : ""}` },
    h(
      "div",
      { class: "row spread", style: "gap:16px" },
      h(
        "div",
        { class: "grow" },
        h(
          "div",
          { class: "row", style: "gap:8px" },
          h("h3", {}, opts.title),
          opts.active && h("span", { class: "tag accent" }, t("engines.banner.defaultBadge")),
        ),
        h("div", { class: "desc" }, opts.desc),
      ),
      !opts.active &&
        opts.onAction &&
        h(
          "button",
          {
            type: "button",
            class: "btn primary sm",
            disabled: opts.actionDisabled,
            title: opts.actionTitle,
            onclick: opts.onAction,
          },
          t("engines.banner.setDefault"),
        ),
    ),
  );
}
