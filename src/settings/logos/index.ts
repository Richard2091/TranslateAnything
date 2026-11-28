/// <reference types="vite/client" />
// 服务商 logo：取自 LobeHub Icons（@lobehub/icons-static-svg，MIT），只拷了用到的几个，构建时内联。
// 文件名即 AI_PRESETS 的 id；商标归各自所有者。单色图标用 currentColor，随深浅色主题变化。
import { h, icon } from "../ui";

const SVGS = import.meta.glob<string>("./*.svg", { query: "?raw", import: "default", eager: true });
const BY_ID: Record<string, string> = Object.fromEntries(Object.entries(SVGS).map(([path, svg]) => [path.slice(2, -4), svg]));

let seq = 0;

/** 同一图标在页面上出现多次时，渐变等 id 会撞车（被隐藏的那份还会让其他份丢色），插入时给 id 加唯一后缀 */
function uniqueIds(svg: string): string {
  const ids = [...svg.matchAll(/\bid="([^"]+)"/g)].map((m) => m[1]);
  if (!ids.length) return svg;
  const n = ++seq;
  for (const id of ids) svg = svg.split(`"${id}"`).join(`"${id}-${n}"`).split(`#${id})`).join(`#${id}-${n})`);
  return svg;
}

/** 服务商 logo 方块；没有对应图标（自定义接口等）时显示通用图标 */
export function providerLogo(presetId: string | undefined): HTMLElement {
  const svg = presetId ? BY_ID[presetId] : undefined;
  const box = h("span", { class: "logo", "aria-hidden": "true" });
  if (svg) box.innerHTML = uniqueIds(svg);
  else box.append(icon("sparkle"));
  if (!svg) box.classList.add("generic");
  return box;
}
