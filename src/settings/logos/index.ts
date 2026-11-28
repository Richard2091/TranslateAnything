/// <reference types="vite/client" />
// 服务商 logo：取自 LobeHub Icons（@lobehub/icons-static-svg，MIT），只拷了用到的几个，构建时内联。
// 文件名即 AI_PRESETS 的 id；商标归各自所有者。单色图标用 currentColor，随深浅色主题变化。
//
// 已配置供应商的图标（ProviderConfig.icon）：留空 = 自动（按接口地址匹配内置 logo，匹配不到取网站图标）；
// `preset:<id>` = 指定内置 logo；`none` = 通用图标；`data:image/...` = 用户上传的图片。
import { invoke } from "@tauri-apps/api/core";
import { h, icon } from "../ui";
import { cfg, type ProviderConfig } from "../state";

const SVGS = import.meta.glob<string>("./*.svg", { query: "?raw", import: "default", eager: true });
const BY_ID: Record<string, string> = Object.fromEntries(Object.entries(SVGS).map(([path, svg]) => [path.slice(2, -4), svg]));

/** 有内置 logo 的预设 id */
export const hasLogo = (presetId: string): boolean => presetId in BY_ID;

let seq = 0;

/** 同一图标在页面上出现多次时，渐变等 id 会撞车（被隐藏的那份还会让其他份丢色），插入时给 id 加唯一后缀 */
function uniqueIds(svg: string): string {
  const ids = [...svg.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1]);
  if (!ids.length) return svg;
  const n = ++seq;
  for (const id of ids) svg = svg.split(`"${id}"`).join(`"${id}-${n}"`).split(`#${id})`).join(`#${id}-${n})`);
  return svg;
}

function box(): HTMLElement {
  return h("span", { class: "logo", "aria-hidden": "true" });
}

function fillGeneric(el: HTMLElement) {
  el.replaceChildren(icon("sparkle"));
  el.classList.add("generic");
}

function fillImage(el: HTMLElement, src: string) {
  const img = h("img", { src, alt: "", draggable: "false" });
  // 图片坏了（缓存的网站图标格式浏览器不认等）就退回通用图标
  img.addEventListener("error", () => fillGeneric(el), { once: true });
  el.replaceChildren(img);
  el.classList.remove("generic");
}

/** 服务商 logo 方块；没有对应图标（自定义接口等）时显示通用图标 */
export function providerLogo(presetId: string | undefined): HTMLElement {
  const svg = presetId ? BY_ID[presetId] : undefined;
  const el = box();
  if (svg) el.innerHTML = uniqueIds(svg);
  else fillGeneric(el);
  return el;
}

// ───────────────────────── 网站图标 ─────────────────────────

/** 按域名记住取过的网站图标（对象 URL），同一域名只请求一次 */
const siteIcons = new Map<string, Promise<string | null>>();

function hostKey(url: string): string {
  try {
    return new URL(url.trim()).host.toLowerCase();
  } catch {
    return "";
  }
}

function siteIcon(id: string, baseUrl: string): Promise<string | null> {
  const host = hostKey(baseUrl);
  if (!host) return Promise.resolve(null);
  let p = siteIcons.get(host);
  if (!p) {
    p = invoke<{ mime: string; bytes: number[] } | null>("provider_site_icon", { id })
      .then((r) => (r ? URL.createObjectURL(new Blob([new Uint8Array(r.bytes)], { type: r.mime })) : null))
      .catch(() => null);
    siteIcons.set(host, p);
  }
  return p;
}

/** 删除供应商、改接口地址后：清掉已没有引擎使用的域名的图标缓存（本页内存里的和磁盘上的） */
export function pruneSiteIcons() {
  const used = new Set(Object.values(cfg().engines ?? {}).map((p) => hostKey(p.base_url ?? "")).filter(Boolean));
  for (const [host, p] of siteIcons) {
    if (used.has(host)) continue;
    siteIcons.delete(host);
    void p.then((u) => u && URL.revokeObjectURL(u));
  }
  void invoke("site_icon_prune").catch(() => {});
}

/**
 * 已配置供应商的图标
 * @param autoPreset 自动模式下按接口地址匹配到的内置预设 id；没匹配到时按用户填的接口地址取网站图标
 */
export function providerIcon(id: string, p: ProviderConfig, autoPreset: string | undefined): HTMLElement {
  const mode = p.icon?.trim() ?? "";
  if (mode === "none") return providerLogo(undefined);
  if (mode.startsWith("preset:")) return providerLogo(mode.slice(7));
  if (mode.startsWith("data:image/")) {
    const el = box();
    fillImage(el, mode);
    return el;
  }
  if (autoPreset && BY_ID[autoPreset]) return providerLogo(autoPreset);
  const el = providerLogo(undefined);
  void siteIcon(id, p.base_url ?? "").then((src) => src && fillImage(el, src));
  return el;
}

// ───────────────────────── 上传图片 ─────────────────────────

/** 选一张本地图片，缩成 64×64 的 PNG（保持比例、居中、透明底），返回 data URL；取消返回 null，读不了返回空串 */
export function pickImage(): Promise<string | null> {
  return new Promise((resolve) => {
    const input = h("input", { type: "file", accept: "image/png,image/jpeg,image/svg+xml,image/webp,image/gif,image/x-icon,.ico" });
    input.addEventListener("cancel", () => resolve(null));
    input.addEventListener("change", () => {
      const file = input.files?.[0];
      if (!file) return resolve(null);
      const url = URL.createObjectURL(file);
      const img = new Image();
      img.onload = () => {
        const size = 64;
        const canvas = h("canvas", { width: size, height: size });
        const ctx = canvas.getContext("2d");
        const w = img.naturalWidth || size;
        const hh = img.naturalHeight || size;
        const k = Math.min(size / w, size / hh);
        ctx?.drawImage(img, (size - w * k) / 2, (size - hh * k) / 2, w * k, hh * k);
        URL.revokeObjectURL(url);
        resolve(ctx ? canvas.toDataURL("image/png") : "");
      };
      img.onerror = () => {
        URL.revokeObjectURL(url);
        resolve("");
      };
      img.src = url;
    });
    input.click();
  });
}
