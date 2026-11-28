// 作用范围（全局 / 某个软件）：过滤规则页与术语表页共用
import { invoke } from "@tauri-apps/api/core";
import { StoreAppSummary, cfg } from "../state";
import { t } from "../../i18n";
import { dropdown } from "../ui";

export const GLOBAL_SCOPE = "__global__";

export interface Scope {
  id: string;
  label: string;
}

/** 译文库里记录过的软件（作用范围的候选） */
let apps: StoreAppSummary[] = [];

/** 与后端 `normalize_app_id` 一致：取路径最后一段并转小写 */
export function normalizeAppId(raw: string): string {
  const s = raw.trim();
  if (!s) return "";
  return (s.split(/[\\/]/).pop() ?? s).trim().toLowerCase();
}

export async function loadScopeApps() {
  apps = (await invoke<StoreAppSummary[]>("store_apps")) ?? [];
}

/** 全局 + 沉浸模式应用 + 已有规则的应用（`ids`）+ 译文库里的软件，按名称排序 */
export function collectScopes(ids: string[]): Scope[] {
  const map = new Map<string, string>();
  for (const tg of cfg().immersive.targets) {
    const id = normalizeAppId(tg.exe) || normalizeAppId(tg.name);
    if (id && !map.has(id)) map.set(id, tg.name.trim() || id);
  }
  for (const key of ids) {
    const id = normalizeAppId(key);
    if (id && !map.has(id)) map.set(id, id);
  }
  for (const a of apps) {
    const id = normalizeAppId(a.id);
    if (id && (!map.has(id) || map.get(id) === id)) map.set(id, a.name.trim() || id);
  }
  return [
    { id: GLOBAL_SCOPE, label: t("filter.scope.global") },
    ...[...map.entries()].sort((a, b) => a[1].localeCompare(b[1])).map(([id, label]) => ({ id, label })),
  ];
}

/** 作用范围下拉（显示名与程序名不同时把程序名作小字提示） */
export function scopeDropdown(scopes: Scope[], value: string, onPick: (v: string) => void): HTMLElement {
  const options = scopes.map((s) => ({ value: s.id, label: s.label, hint: s.id !== GLOBAL_SCOPE && s.label !== s.id ? s.id : undefined }));
  return dropdown(options, value, onPick, { title: t("filter.scope.label") });
}
