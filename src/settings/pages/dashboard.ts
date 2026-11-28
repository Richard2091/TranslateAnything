// 概览：仪表盘式首页，工作模式 + 翻译链路（逐节点测速） + 语言与网络 + 译文库
import {
  S,
  aiChain,
  applyMethod,
  cfg,
  effectiveChain,
  isLLM,
  kindOf,
  langs,
  methodOf,
  providerLabel,
  saveConfig,
  startChainTest,
} from "../state";
import { attachPopover, cleanupStalePopovers, dropdown, fmt, fmtSec, h, icon, keys, pageHead, segmented, switchEl, toast, type Child, type IconName } from "../ui";
import { go } from "../nav";
import { t } from "../../i18n";
import { setMasterSwitch, toggleAllButton } from "./immersive";
import { closeAllFrames, startSelect } from "./overlay";
import { defaultBase, hostOf, protocolLabel, proxyField } from "./engineShared";

function tile(ic: IconName, title: string, right: Child, ...body: Child[]): HTMLElement {
  return h(
    "div",
    { class: "card tile" },
    h("div", { class: "tile-head" }, h("span", { class: "tile-icon" }, icon(ic)), h("h3", { class: "grow" }, title), right),
    ...body,
  );
}

const more = (tab: string, label = t("dash.details")) => h("button", { class: "link-btn", onclick: () => go(tab) }, label, icon("arrow"));

function renderChainNodePopover(id: string, nodeState: "idle" | "testing" | "ok" | "error" | "skipped"): HTMLElement {
  const c = cfg();
  const name = providerLabel(c, id);
  const step = S.chainTest.steps[id];
  const llm = isLLM(c, id);
  const p = c.engines?.[id];
  const kind = kindOf(c, id);

  let dotCls = "";
  let statusText = name;
  if (nodeState === "testing") {
    dotCls = " accent";
    statusText = `${name} · ${t("engines.test.busy")}`;
  } else if (nodeState === "ok" && step) {
    dotCls = " ok";
    statusText = `${t("dash.chain.ok")} · ${fmtSec(step.ms ?? 0)}`;
  } else if (nodeState === "error" && step) {
    dotCls = " bad";
    statusText = `${t("dash.chain.fail")} · ${fmtSec(step.ms ?? 0)}`;
  } else if (nodeState === "skipped") {
    statusText = t("dash.chain.skipped");
  } else if (S.cooldowns.includes(id)) {
    dotCls = " warn";
    statusText = `${name} · ${t("engines.tag.cooling")}`;
  }

  const meta = llm
    ? `${protocolLabel(kind)} · ${p?.model?.trim() || t("dash.noModel")}`
    : hostOf(p?.base_url || defaultBase(kind, p ?? { base_url: "", api_key: "", region: "", model: "", proxy: null })) || t("dash.builtinEngine");

  const body: Child[] = [];
  if (nodeState === "error" && step?.error) {
    body.push(h("div", { class: "hover-pop-body bad" }, step.error));
  } else if (nodeState === "ok" && step) {
    body.push(h("div", { class: "hover-pop-body ok" }, `${step.source ?? ""}\n→ ${step.text ?? ""}`));
  } else if (nodeState === "skipped") {
    body.push(h("div", { class: "hover-pop-body" }, t("dash.chain.skippedHint")));
  } else if (S.cooldowns.includes(id)) {
    body.push(h("div", { class: "hover-pop-body" }, t("dash.chain.coolingHint")));
  }

  return h(
    "div",
    { class: "hover-pop-inner" },
    h("div", { class: "hover-pop-head" }, h("span", { class: `hover-pop-dot${dotCls}` }), h("b", {}, statusText)),
    h("div", { class: "hover-pop-meta" }, meta),
    ...body,
  );
}

function chainNodeEl(id: string, nodeState: "idle" | "testing" | "ok" | "error" | "skipped"): HTMLElement {
  const c = cfg();
  const name = providerLabel(c, id);
  const step = S.chainTest.steps[id];
  const cooling = nodeState === "idle" && S.cooldowns.includes(id);

  const cls = [
    "chain-node",
    nodeState !== "idle" ? nodeState : "",
    cooling ? "cooling" : "",
  ]
    .filter(Boolean)
    .join(" ");

  const el = h(
    "button",
    { type: "button", class: cls },
    nodeState === "testing" && h("span", { class: "chain-spin", "aria-hidden": "true" }),
    cooling && h("span", { class: "chain-cool-dot", "aria-hidden": "true" }),
    h("span", { class: "chain-name" }, name),
    nodeState === "ok" && step?.ms != null && h("span", { class: "chain-dur" }, fmtSec(step.ms)),
  );
  attachPopover(el, () => renderChainNodePopover(id, nodeState));
  return el;
}

export function renderDashboard(root: HTMLElement) {
  const c = cfg();
  const im = c.immersive;
  const attached = S.statuses.filter((s) => s.state === "attached");
  const translated = attached.reduce((n, s) => n + s.translated, 0);
  const method = methodOf(c);
  const aiList = aiChain(c);
  const chain = effectiveChain(c);

  // ── 悬浮窗模式 ──
  const overlay = tile(
    "frame",
    t("nav.overlay"),
    more("overlay"),
    h("div", { class: "tile-big" }, h("b", {}, String(S.frames)), h("span", {}, t("dash.framesOpen"))),
    h("div", { class: "tile-line" }, t("dash.hotkey"), " ", keys(c.hotkey)),
    h(
      "div",
      { class: "tile-actions" },
      h("button", { class: "btn primary sm", onclick: startSelect }, icon("frame"), t("overlayPage.start")),
      h("button", { class: "btn ghost sm", disabled: S.frames === 0, onclick: closeAllFrames }, t("overlayPage.closeAll")),
    ),
  );

  // ── 沉浸模式 ──
  const immersive = tile(
    "apps",
    t("nav.immersive"),
    switchEl(im.enabled, setMasterSwitch),
    h(
      "div",
      { class: "tile-big" },
      h("b", {}, `${attached.length}`),
      h("span", {}, im.enabled ? t("dash.appsAttached", { n: im.targets.length }) : t("dash.masterOff")),
    ),
    h("div", { class: "tile-line" }, attached.length ? t("dash.translatedThisRun", { n: fmt(translated) }) : t("dash.attachHint")),
    h("div", { class: "tile-actions" }, toggleAllButton("btn sm"), h("span", { class: "spacer" }), more("immersive", t("dash.manageApps"))),
  );

  // ── 翻译链路（通栏） ──
  const hasOk = Object.values(S.chainTest.steps).some((s) => s.status === "ok");
  const testAllBtn = h(
    "button",
    {
      type: "button",
      class: "btn sm",
      disabled: S.chainTest.busy,
      title: t("dash.chain.testHint"),
      onclick: () => void startChainTest(),
    },
    icon("bolt"),
    S.chainTest.busy ? t("engines.test.busy") : t("dash.chain.test"),
  );

  const flowChildren: Child[] = [];
  let dividerInserted = false;
  chain.forEach((id, idx) => {
    if (method === "llm" && !dividerInserted && !isLLM(c, id)) {
      dividerInserted = true;
      flowChildren.push(h("span", { class: "chain-arrow" }, icon("arrow")));
      flowChildren.push(h("span", { class: "chain-divider" }, t("dash.backup")));
    } else if (idx > 0) {
      flowChildren.push(h("span", { class: "chain-arrow" }, icon("arrow")));
    }

    const step = S.chainTest.steps[id];
    let nodeState: "idle" | "testing" | "ok" | "error" | "skipped" = "idle";
    if (step) nodeState = step.status === "testing" ? "testing" : step.status === "ok" ? "ok" : "error";
    else if ((S.chainTest.finished || hasOk) && S.chainTest.runId) nodeState = "skipped";

    flowChildren.push(chainNodeEl(id, nodeState));
  });

  const chainCard = h(
    "div",
    { class: "card tile tile-wide" },
    h(
      "div",
      { class: "tile-head chain-head" },
      h("span", { class: "tile-icon" }, icon("engine")),
      h("h3", {}, t("dash.chain")),
      segmented(
        [
          ["engine", t("dash.methodEngine")],
          ["llm", t("dash.methodLlm")],
        ],
        method,
        (m) => {
          if (m === "llm" && aiList.length === 0) {
            toast(t("dash.needAiProvider"), true);
            go("ai");
            return;
          }
          void saveConfig((x) => { applyMethod(x, m); }, m === "llm" ? t("dash.switchedLlm") : t("dash.switchedEngine"));
        },
      ),
      h("span", { class: "spacer" }),
      testAllBtn,
      more(method === "llm" ? "ai" : "mt", t("dash.configure")),
    ),
    h("div", { class: "chain-flow" }, ...flowChildren),
    h(
      "div",
      { class: "tile-line" },
      method === "llm" ? t("dash.chain.aiHint") : t("dash.chain.mtHint"),
    ),
  );

  // ── 语言与网络 ──
  const languageNet = tile(
    "globe",
    t("dash.langNet"),
    more("general", t("dash.adjust")),
    dropdown(
      langs().map(([v, n]) => ({ value: v, label: n, hint: v })),
      c.target_lang,
      (v) => saveConfig((x) => (x.target_lang = v), t("general.targetLangChanged")),
      { width: "100%" },
    ),
    h(
      "div",
      { class: "proxy-summary" },
      h("span", { class: "muted" }, t("general.proxy")),
      proxyField(c.proxy ?? "", null, (val) => saveConfig((x) => (x.proxy = val))),
    ),
  );

  // ── 译文库与术语 ──
  const st = S.stats;
  const store = tile(
    "db",
    t("dash.store"),
    more("library"),
    h(
      "div",
      { class: "tile-metrics" },
      h("div", {}, h("b", {}, st ? fmt(st.total) : "—"), h("span", {}, t("dash.entries"))),
      h("div", {}, h("b", {}, st ? fmt(st.hits) : "—"), h("span", {}, t("dash.hits"))),
      h("div", {}, h("b", {}, String(Object.keys(c.glossary).length)), h("span", {}, t("dash.terms"))),
    ),
    h("div", { class: "tile-line" }, c.persist_cache ? t("dash.storeOn") : t("dash.storeOff")),
  );

  root.replaceChildren(
    pageHead(t("dash.lead")),
    h("div", { class: "tiles" }, overlay, immersive, chainCard, languageNet, store),
  );
  cleanupStalePopovers();
}
