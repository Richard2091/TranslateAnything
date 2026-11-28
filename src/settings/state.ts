// 设置窗口的数据：类型、全局状态、保存配置、翻译方式换算
import { invoke } from "@tauri-apps/api/core";
import { cleanupStalePopovers, clone, errText, keepSwitchMotion, toast } from "./ui";
import { langLabel, t } from "../i18n";

// ───────────────────────── 类型 ─────────────────────────

export interface FilterRuleSet {
  keep_patterns: string[];
  force_translate: string[];
}

export interface FilterConfig {
  keep_patterns: string[];
  force_translate: string[];
  apps: Record<string, FilterRuleSet>;
}

export interface ImmersiveTarget {
  name: string;
  enabled: boolean;
  exe: string;
  args: string;
  port: number;
  url_prefixes: string[];
  skip_selectors: string[];
  auto_relaunch: boolean;
  filter?: FilterRuleSet;
}

export interface ProviderConfig {
  name?: string;
  type?: string;
  base_url: string;
  api_key: string;
  region: string;
  model: string;
  proxy: string | null;
  concurrency?: number | null;
  /** 图标：留空 = 自动；preset:<id> = 内置 logo；none = 通用图标；data:image/... = 上传的图片（见 logos/index.ts） */
  icon?: string;
}

export interface Config {
  hotkey: string;
  /** 新建翻译框的默认鼠标操作：true 为穿透，false 为拖动 */
  frame_click_through: boolean;
  frame_corner_radius: number;
  target_lang: string;
  mode: string;
  ocr_lang: string;
  engine: string;
  fallback: string[];
  ai_chain?: string[];
  engines: Record<string, ProviderConfig>;
  /** 全局代理：空 = 直连；system = 系统代理；其他 = 代理地址 */
  proxy?: string;
  persist_cache: boolean;
  builtin_glossary: boolean;
  glossary: Record<string, string>;
  /** 按软件的自定义术语：应用标识 → 术语表 */
  glossary_apps?: Record<string, Record<string, string>>;
  filter: FilterConfig;
  immersive: { enabled: boolean; hotkey: string; targets: ImmersiveTarget[] };
  /** 界面语言；空或缺省表示跟随系统 */
  ui_lang?: string;
  update: { auto_check: boolean; skip_version?: string; channel?: string };
  [key: string]: unknown;
}

export interface SettingsView {
  config: Config;
  config_path: string;
  config_size: number;
  db_path: string;
  db_size: number;
  db_custom: boolean;
  builtin_glossary: Record<string, string>;
}

export type StateName = "off" | "disabled" | "not_running" | "no_port" | "conflict" | "waiting" | "attached";

export interface TargetStatus {
  port: number;
  name: string;
  enabled: boolean;
  state: StateName;
  message: string;
  pages: { id: string; url: string; title: string }[];
  show: boolean;
  translated: number;
  engine_batches: number;
}

export interface EngineInfo {
  id: string;
  name: string;
  needs_key: boolean;
  official: boolean;
  fields: string[];
  note: string;
  /** 接口地址留空时实际使用的地址；没有默认值的为空 */
  default_base_url: string;
}

export interface AppVersion {
  version: string;
  arch: string;
  portable: boolean;
  repo: string;
  mirrors: string[];
}

export interface UpdateInfo {
  current: string;
  latest: string;
  newer: boolean;
  url: string;
  published_at: string;
  /** 更新说明（Release 正文，上中文下英文的 Markdown） */
  notes: string;
  asset_name: string | null;
  asset_size: number;
  via: string;
}

export interface StoreStats {
  total: number;
  manual: number;
  hits: number;
}

export interface StoreAppSummary {
  id: string;
  name: string;
  count: number;
}

export interface FilterEvent {
  text: string;
  decision: string;
  reason: string;
  app_id: string;
  app_name: string;
  mode: "overlay" | "immersive";
  at: number;
}

export interface EngineTest {
  engine: string;
  source: string;
  text: string;
  ms: number;
}

export interface ChainTestStep {
  run_id: string;
  id: string;
  name: string;
  status: "testing" | "ok" | "error";
  ms: number | null;
  source: string | null;
  text: string | null;
  error: string | null;
}

// ───────────────────────── 全局状态 ─────────────────────────

export const S = {
  view: undefined as unknown as SettingsView,
  statuses: [] as TargetStatus[],
  catalog: [] as EngineInfo[],
  stats: null as StoreStats | null,
  frames: 0,
  ocrLangs: null as { tag: string; name: string }[] | null,
  version: null as AppVersion | null,
  /** 最近一次检查更新的结果；checking / installing 为进行中，progress 为下载进度（百分比） */
  update: {
    info: null as UpdateInfo | null,
    checking: false,
    installing: false,
    progress: 0,
    error: "",
  },
  /** 各引擎最近一次测试结果（按引擎 id） */
  tests: {} as Record<string, { busy: boolean; result?: EngineTest; error?: string }>,
  /** 处于失败冷却期的引擎 id */
  cooldowns: [] as string[],
  /** 首页翻译链路逐节点测速状态 */
  chainTest: {
    runId: "",
    busy: false,
    finished: false,
    steps: {} as Record<string, ChainTestStep>,
  },
};

export const cfg = () => S.view.config;

let rerender: () => void = () => {};
export function setRerender(fn: () => void) {
  rerender = fn;
}

/** 重绘界面，并尽量把焦点还给重绘前聚焦的那个控件（按 data-k 对应） */
export function refresh() {
  const key = (document.activeElement as HTMLElement | null)?.dataset?.k;
  const resumeSwitch = keepSwitchMotion();
  rerender();
  resumeSwitch();
  cleanupStalePopovers();
  if (key) document.querySelector<HTMLElement>(`[data-k="${key}"]`)?.focus({ preventScroll: true });
}

export function resetChainTest() {
  S.chainTest = { runId: "", busy: false, finished: false, steps: {} };
}

export async function loadCooldowns() {
  try {
    S.cooldowns = await invoke<string[]>("engine_cooldowns");
  } catch {
    S.cooldowns = [];
  }
}

export async function loadView() {
  S.view = await invoke<SettingsView>("settings_get");
}

export async function loadStats() {
  try {
    S.stats = await invoke<StoreStats>("store_stats");
  } catch {
    S.stats = null;
  }
}

let lastSave: Promise<unknown> = Promise.resolve();

/** 等正在进行的保存完成（后端按已保存的配置干活前调用，如获取模型列表） */
export const settled = () => lastSave;

let pending = 0;
/** 是否有保存还没完成（此时 `cfg()` 可能还是改动前的配置） */
export const saving = () => pending > 0;

/** 顺序保存配置，连续操作从上一次保存后的配置开始。 */
export function saveConfig(
  mutate: (c: Config) => void,
  okMsg: string = t("common.saved"),
  { redraw = true } = {},
): Promise<boolean> {
  pending++;
  const p = lastSave.then(() => doSave(mutate, okMsg, redraw)).finally(() => pending--);
  lastSave = p.catch(() => {});
  return p;
}

async function doSave(mutate: (c: Config) => void, okMsg: string, redraw: boolean): Promise<boolean> {
  const next = clone(S.view.config);
  mutate(next);
  try {
    await invoke("settings_save", { config: next });
    await loadView();
    S.cooldowns = [];
    resetChainTest();
    if (redraw) refresh();
    if (okMsg) toast(okMsg);
    return true;
  } catch (e) {
    toast(errText(e), true);
    if (redraw) refresh();
    return false;
  }
}

/** 可选的目标语言标签；名称按界面语言现取（Intl.DisplayNames） */
export const LANG_TAGS = ["zh-CN", "zh-TW", "en", "ja", "ko", "fr", "de", "es", "pt", "it", "ru"];

// zh-CN / zh-TW 按地区显示是“中文（中国）”，改用文字标签显示成“简体中文 / 繁体中文”
const DISPLAY_TAG: Record<string, string> = { "zh-CN": "zh-Hans", "zh-TW": "zh-Hant" };

export const langName = (tag: string) => langLabel(DISPLAY_TAG[tag] ?? tag);

/** [标签, 名称]；名称依赖界面语言，所以每次调用时现算 */
export const langs = (): [string, string][] => LANG_TAGS.map((tag) => [tag, langName(tag)]);

// ───────────────────────── 翻译方式与调用链 ─────────────────────────
// 配置里只有"主用引擎 + 回退顺序"以及"AI 启用顺序（ai_chain）"。
// 界面上的"翻译方式"由主用引擎类型推出：主用的是 AI（openai / anthropic / gemini）即为 AI 翻译，否则为机器翻译。

export const AI_KINDS = new Set(["openai", "anthropic", "gemini"]);

export const kindOf = (c: Config, id: string) => (c.engines?.[id]?.type || "").trim() || id;
/** 新建的自定义接口还没选协议：AI 实例 id 都以 ai_ 开头，类型留空 */
export const isUnsetAi = (c: Config, id: string) => id.startsWith("ai_") && !(c.engines?.[id]?.type || "").trim();
export const isLLM = (c: Config, id: string) => AI_KINDS.has(kindOf(c, id)) || isUnsetAi(c, id);
export const engineInfo = (kind: string) => S.catalog.find((e) => e.id === kind);
export const providerLabel = (c: Config, id: string) =>
  c.engines?.[id]?.name?.trim() || (isUnsetAi(c, id) ? t("engines.preset.custom") : engineInfo(kindOf(c, id))?.name || id);
export const engineLabel = providerLabel;

/** 配置中所有的 AI 供应商实例 id */
export function aiIds(c: Config): string[] {
  return Object.keys(c.engines ?? {}).filter((id) => isLLM(c, id));
}

/** 已启用的 AI 供应商按顺序排列 */
export function aiChain(c: Config): string[] {
  const ids = aiIds(c);
  if (!ids.length) return [];
  const raw = c.ai_chain;
  if (raw && raw.length > 0) {
    return raw.map((s) => s.trim()).filter((id, i, a) => ids.includes(id) && a.indexOf(id) === i);
  }
  // 兼容旧配置：未写 ai_chain 时，从当前主用与回退中提取 AI 实例
  return [c.engine, ...c.fallback]
    .map((s) => s.trim())
    .filter((id, i, a) => id && ids.includes(id) && a.indexOf(id) === i);
}

export type Method = "engine" | "llm";
export const methodOf = (c: Config): Method => (isLLM(c, c.engine) ? "llm" : "engine");

/** 传统机器翻译引擎的尝试顺序（主用 + 回退，去掉 AI），没有就用 edge */
export function engineChain(c: Config): string[] {
  const chain = [c.engine, ...c.fallback].map((s) => s.trim()).filter((id, i, a) => id && !isLLM(c, id) && a.indexOf(id) === i);
  return chain.length ? chain : ["edge"];
}

/** 当前实际生效的完整调用链（与后端 Config::engine_chain 一致） */
export function effectiveChain(c: Config): string[] {
  const mt = engineChain(c);
  if (methodOf(c) === "llm") {
    const ai = aiChain(c);
    if (ai.length) return [...ai, ...mt];
  }
  return mt;
}

/**
 * 切换默认翻译方式。
 * - 切到 AI 翻译：需要至少已启用一个 AI 供应商；将已启用的 AI 链放前面，机器翻译链放后面作后备。
 * - 切回机器翻译：保留当前的 ai_chain，将机器翻译链恢复为主用 + 回退。
 */
export function applyMethod(c: Config, m: Method): boolean {
  const mt = engineChain(c);
  const ai = aiChain(c);
  c.ai_chain = ai;
  if (m === "llm") {
    if (!ai.length) return false;
    c.engine = ai[0];
    c.fallback = [...ai.slice(1), ...mt];
    return true;
  }
  c.engine = mt[0];
  c.fallback = mt.slice(1);
  return true;
}

/** 写回机器翻译顺序：默认方式为机器翻译时直接作主链，为 AI 翻译时作后备 */
export function setEngineChain(c: Config, ids: string[]) {
  if (methodOf(c) === "llm") {
    const ai = aiChain(c);
    c.ai_chain = ai;
    if (ai.length) {
      c.engine = ai[0];
      c.fallback = [...ai.slice(1), ...ids];
      return;
    }
  }
  c.engine = ids[0];
  c.fallback = ids.slice(1);
}

/** 写回 AI 供应商启用顺序；当前处于 AI 翻译时同步更新生效链，全部停用则切回机器翻译 */
export function setAiChain(c: Config, ids: string[]) {
  const mt = engineChain(c);
  c.ai_chain = ids;
  if (methodOf(c) === "llm") {
    if (ids.length > 0) {
      c.engine = ids[0];
      c.fallback = [...ids.slice(1), ...mt];
    } else {
      c.engine = mt[0];
      c.fallback = mt.slice(1);
    }
  }
}

/** 测试某个引擎（用已保存的配置），结果记到 S.tests 并刷新界面；返回是否通过 */
export async function testEngine(id: string): Promise<boolean> {
  S.tests[id] = { busy: true };
  refresh();
  let ok = false;
  try {
    S.tests[id] = { busy: false, result: await invoke<EngineTest>("engine_test", { id }) };
    S.cooldowns = S.cooldowns.filter((x) => x !== id);
    ok = true;
  } catch (e) {
    S.tests[id] = { busy: false, error: errText(e) };
  }
  refresh();
  return ok;
}

/** 首页链路逐节点测速 */
export async function startChainTest() {
  if (S.chainTest.busy) return;
  const runId = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  S.chainTest = { runId, busy: true, finished: false, steps: {} };
  refresh();
  try {
    const steps = await invoke<ChainTestStep[]>("engine_test_chain", { runId });
    if (S.chainTest.runId !== runId) return;
    for (const step of steps) S.chainTest.steps[step.id] = step;
    await loadCooldowns();
  } catch (e) {
    toast(errText(e), true);
  } finally {
    if (S.chainTest.runId === runId) {
      S.chainTest.busy = false;
      S.chainTest.finished = true;
      refresh();
    }
  }
}
