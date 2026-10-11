//! OpenAPI 3.0 文档生成逻辑（openapi-external-access）——**单一实现**。
//!
//! 从 `Pre-Proc/{namespace}/Sources/Apps/Services/*/service.json` 半自动生成
//! OAS3 文档，产物 `Pre-Proc/{namespace}/openapi.json`（写入归 CLI 壳
//! `scripts/openapi-generate.ts`；门禁 `scripts/check/check-openapi-doc-drift.ts`
//! 与本模块同 import 重生成比对——MUST NOT 存在第二份生成逻辑）。
//!
//! 端点约定（对齐 Gateway service_registry）：
//!   GET    /api/service/{service}/{entity}            → 列表（read:{entity} scope）
//!   POST   /api/service/{service}/{entity}            → 创建（create:{entity}）
//!   GET    /api/service/{service}/{entity}/{id}       → 单条（read:{entity}）
//!   PUT    /api/service/{service}/{entity}/{id}       → 更新（update:{entity}）
//!   DELETE /api/service/{service}/{entity}/{id}       → 删除（delete:{entity}）
//!
//! 每个操作标注 OAuth2 security scheme 与所需 scope（PEP scope 强制对齐：
//! `read:{entity}` / `create:{entity}` / `update:{entity}` / `delete:{entity}`）。

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { gatewaySourcesKindDir } from "./preproc-layout.mjs";

/// 仓库根（相对本模块位置；MUST NOT 依赖调用方 cwd）。
/// `scripts/lib/` 上溯两级 = 仓根（平台仓；ns 仓档位下由调用方显式传 `root`）。
export const REPO_ROOT = resolve(import.meta.dirname, "..", "..");

/// 动作清单条目契约（openapi-actions.json items；direction 见 fix-wz-openapi-actions-drift）。
/// `scope` 字段**已退役**：inbound 动作的 scope 由生成器按 PEP 同规则推导
/// （`method_to_action(method)` × 路径首段实体名下划线化，对齐 `pep/middleware.rs`
/// 的 `parts[2].replace('-', '_')`）——清单不再自持第二份 scope 推导。
export type ActionItem = {
  path?: string;
  direction?: "inbound" | "outbound";
  method?: string;
  summary?: string;
  operationId?: string;
  entity?: string;
  parameters?: unknown[];
  requestBody?: unknown;
  externalBase?: string;
  trigger?: string;
  /**
   * 出向条目（`direction: "outbound"`）的 provider 派发键——**必填**。
   *
   * 取值 = `Ext-adapter/<ext>` 目录代号 = `OutboundProvider::provider_id()` =
   * `isahl_auth.outbound_client.provider`（单一代号，禁第二份映射表）。缺省时生成
   * **fail-closed**（`OpenapiDocError`）；`provider` 原样进入产物 `x-outbound-contracts`。
   *
   * 规约：`openspec/specs/openapi-external-access :: oas3-outbound-contract-provider-binding`；
   * 跨面连通性（适配器 / L-MAP / 契约）由门禁 `check-provider-code-alignment` 判定。
   */
  provider?: string;
};

type ActionEntry = { service: string; items: ActionItem[] };

/** 产物文档（写盘形态 = `JSON.stringify(doc, null, 2) + '\n'`，门禁逐字节比对同源）。 */
export interface OpenapiDoc {
  openapi: string;
  info: { title: string; version: string; description: string };
  servers: Array<{ url: string }>;
  tags: Array<{ name: string }>;
  paths: Record<string, unknown>;
  "x-outbound-contracts"?: ActionItem[];
  components: Record<string, unknown>;
}

/**
 * 生成器判定失败（fail-closed）：CLI 壳打印 `message` 后 exit 1；
 * 门禁把它收敛为一条违规（不 exit 整个门禁进程）。
 */
export class OpenapiDocError extends Error {}

export interface GenerateOptions {
  /** 仓库根覆盖（自测夹具 / ns 仓档位；缺省 = 本模块所在仓根）。 */
  root?: string;
}

/// 默认产物路径（CLI `--out` 缺省；门禁比对目标同源）。
export function defaultOutputPath(
  namespace: string,
  root: string = REPO_ROOT,
): string {
  return join(root, "Pre-Proc", namespace, "openapi.json");
}

// ═══════════════════════════════════════════════════════════════════════════
// MUST 出现在对应 service handler 源码的 `web::resource("…")` / `.route("…")`
// 参数中（字符串级匹配）。outbound 动作（WZ → 第三方）跳过校验。
// 漂移 → 抛错（fail-closed），杜绝幽灵路径静默入 OAS3 文档。
// ═══════════════════════════════════════════════════════════════════════════
function collectHandlerSource(
  root: string,
  ns: string,
  serviceId: string,
): string {
  return collectSourceTree(
    join(
      gatewaySourcesKindDir(root, ns, "Services"),
      serviceId,
      "backend",
      "src",
    ),
  );
}

/// 薄包装服务（backend/src 无路由注册，lib.rs 引用 Framework crate 的
/// configure_routes/register_service_routes）→ 跟随 Framework crate 源码提取。
/**
 * service 路由源码集合（单一来源，生成器与覆盖门禁同消费）：
 * service 自身 `backend/src` ∪ （薄壳委托时）lib.rs 引用的 Framework 内核 crate 源码。
 */
export function collectServiceRouteSources(
  root: string,
  ns: string,
  serviceId: string,
): string[] {
  const own = collectHandlerSource(root, ns, serviceId);
  if (extractRoutes(own).size > 0) return [own];
  // 薄包装：lib.rs 引用 <crate>::configure_routes / register_service_routes
  const lib = join(
    gatewaySourcesKindDir(root, ns, "Services"),
    serviceId,
    "backend",
    "src",
    "lib.rs",
  );
  if (!existsSync(lib)) return [own];
  const libText = readFileSync(lib, "utf-8");
  const crateNames = new Set<string>();
  for (const m of libText.matchAll(
    /([a-z][a-z0-9_-]*)::(?:configure_routes|register_service_routes)/g,
  )) {
    crateNames.add(m[1]);
  }
  const texts = [own];
  for (const c of crateNames) {
    // crate 名 → Framework/backend/<dir>（按 Cargo.toml [package].name 匹配）
    const fwRoot = join(root, "Framework", "backend");
    if (!existsSync(fwRoot)) continue;
    for (const dirName of readdirSync(fwRoot, { withFileTypes: true })) {
      if (!dirName.isDirectory()) continue;
      const cargoFile = join(fwRoot, dirName.name, "Cargo.toml");
      if (!existsSync(cargoFile)) continue;
      const cargoText = readFileSync(cargoFile, "utf-8");
      if (cargoText.includes(`name = "${c}"`)) {
        texts.push(collectSourceTree(join(fwRoot, dirName.name, "src")));
      }
    }
  }
  return texts;
}

/// 路由集合 = **字面量 ∪ 结构化全路径**（动作清单交叉校验消费：作用域包裹的声明路径
/// 也须可匹配；空串 = 服务根保留）。
function collectHandlerRoutes(
  root: string,
  ns: string,
  serviceId: string,
): Set<string> {
  const routes = new Set<string>();
  for (const text of collectServiceRouteSources(root, ns, serviceId)) {
    for (const r of extractRoutes(text)) routes.add(r);
    for (const e of extractRouteEntries(text)) {
      routes.add(e.path);
      if (!e.path.startsWith("/")) routes.add(`/${e.path}`);
    }
  }
  return routes;
}

/// 结构化路由条目（作用域感知）：service 自身 src ∪ 薄壳委托的 Framework 内核 src。
/// 实体 CRUD 判定与兜底路径生成的单一真相源（change `fix-openapi-doc-coverage`）。
function collectHandlerRouteEntries(
  root: string,
  ns: string,
  serviceId: string,
): RouteEntry[] {
  const merged = new Map<string, Set<string>>();
  for (const text of collectServiceRouteSources(root, ns, serviceId)) {
    for (const e of extractRouteEntries(text)) {
      const key = normalizeRoutePath(e.path);
      const set = merged.get(key) ?? new Set<string>();
      for (const m of e.methods) set.add(m);
      merged.set(key, set);
    }
  }
  return [...merged.entries()].map(([path, methods]) => ({
    path,
    methods: [...methods],
  }));
}

/// 从给定源码文本提取路由注册参数（**字面量集合**；动作清单交叉校验与覆盖门禁消费）。
function extractRoutes(src: string): Set<string> {
  const routes = new Set<string>();
  // 形态 1：web::resource("X") / .route("X" / cfg.route("X"
  for (const m of src.matchAll(/(?:resource|route)\(\s*"([^"]*)"/g)) {
    routes.add(m[1]);
  }
  // 形态 2：crud_routes::<…>("X") 尾部路径参数
  for (const m of src.matchAll(/>\(\s*"([^"]*)"/g)) {
    routes.add(m[1]);
  }
  return routes;
}

// ═══════════════════════════════════════════════════════════════════════════
// 结构化路由抽取（change `fix-openapi-doc-coverage`）
//
// 规约：`openspec/specs/openapi-external-access :: oas3-doc-live-route-coverage`。
//
// 旧实现（上面的 `extractRoutes`）是**纯字面量枚举** ⇒ 丢失 `web::scope("S")` 前缀语义：
//   web::scope("/customers").route("", web::get()…) / .route("/{id}", web::put()…)
// 的实有 CRUD 在文档中**零路径**（实测 Alioth `customer` 与 118 条 skip 警告）。
//
// 本抽取器 = 受限词法扫描 + 显式链/作用域栈（Rust 无内嵌解析器可用）：
//   · 作用域 `web::scope("S")` → 前缀入栈（`/service/{id}` 挂载前缀**剥离**）；
//   · 条目 `web::resource("R")` / `.route("P", …)` / `.route(handler)` → 路径 + 绑定方法；
//   · `crud_routes::<…>("S")` → 按 crud crate 展开（`/S` get+post、`/S/{id}` get/put/delete、
//     `/S/batch` delete；见 `Framework/backend/crud/src/handler.rs:1097`）；
//   · **无法判定的形态不产出条目**（宁缺勿滥：漏报由 `check-openapi-doc-coverage` 点名，
//     MUST NOT 发明幽灵路径污染对外合同）。
// ═══════════════════════════════════════════════════════════════════════════

/** 实有路由条目：`path` 相对 `/api/service/{service}`（`""` = 服务根），`methods` 为小写动词。 */
export interface RouteEntry {
  path: string;
  methods: string[];
}

/** HTTP 方法名 → 是否为 `web::<method>()` 形态。 */
const ROUTE_METHOD_NAMES = new Set([
  "get",
  "post",
  "put",
  "patch",
  "delete",
  "head",
  "options",
  "trace",
  "connect",
]);

/**
 * 挂载前缀形（`/service/{id}`）：**保留在条目路径中**——文档路径 = `/api` + 注册路径，
 * 与运行时一致（服务自带挂载：`lib.rs` 的 `web::scope("/service/{id}")`，或 handler 内
 * 直接写全路径，如 `crud_routes::<…>("/service/bin-locations")`；实测 CT 服务的挂载段
 * 与 service.json id 不同名 ⇒ 不得用 serviceId 合成）。
 */
const MOUNT_HEAD_RE = /^\/service\/[^/]+/;

/** 条目路径 → 文档路径（`/service/…` 头保留；相对路径按 service 挂载约定补前缀）。 */
function docPathFor(serviceId: string, entryPath: string): string {
  const p = normalizeRoutePath(entryPath);
  if (p.startsWith("/service/")) return `/api${p}`;
  return `/api/service/${serviceId}${p}`;
}

/** 条目路径的**挂载相对**形（实体段判定用；无挂载头 ⇒ 原样）。 */
function mountRelative(entryPath: string): string {
  return normalizeRoutePath(
    normalizeRoutePath(entryPath).replace(MOUNT_HEAD_RE, ""),
  );
}

/**
 * PEP 同规则 scope 实体名（`common::openapi_scope::service_entity_from_path` 镜像）：
 * `/api/service/{svc}/{entity}[/…]` ⇒ `{entity}`（`-` → `_`）；
 * `/api/service/{svc}`（服务根）⇒ `service_{svc}`。生成侧与 PEP 唯此一处派生。
 */
function scopeEntityFromDocPath(docPath: string): string {
  const segs = docPath
    .replace(/^\/api\/service\/?/, "")
    .split("/")
    .filter((s) => s !== "");
  const rt =
    segs.length >= 2 ? (segs[1] as string) : `service_${segs[0] ?? ""}`;
  return rt.replace(/-/g, "_");
}

type RtTok = { kind: "ident" | "str" | "punct"; value: string; depth: number };

/**
 * 受限词法扫描：跳过注释（含嵌套块注释）与字符串正文（raw/byte 字符串整体跳过），
 * 保留标识符、字符串值与标点；`depth` = 该 token **之前**的定界符深度。
 */
function tokenizeRust(src: string): RtTok[] {
  const toks: RtTok[] = [];
  let depth = 0;
  let i = 0;
  const isIdentStart = (c: string): boolean => /[A-Za-z_]/.test(c);
  const isIdentPart = (c: string): boolean => /[A-Za-z0-9_]/.test(c);
  while (i < src.length) {
    const c = src[i] as string;
    const c2 = src[i + 1];
    if (c === "/" && c2 === "/") {
      const nl = src.indexOf("\n", i);
      if (nl < 0) break;
      i = nl + 1;
      continue;
    }
    if (c === "/" && c2 === "*") {
      let lvl = 1;
      i += 2;
      while (i < src.length && lvl > 0) {
        if (src[i] === "/" && src[i + 1] === "*") {
          lvl += 1;
          i += 2;
        } else if (src[i] === "*" && src[i + 1] === "/") {
          lvl -= 1;
          i += 2;
        } else {
          i += 1;
        }
      }
      continue;
    }
    // raw / byte / byte-raw 字符串：r"…" / r#"…"# / br##"…"## / b"…" —— 正文不参与判定
    if (c === "r" || c === "b") {
      const m = /^(?:br|rb|b|r)(#*)"/.exec(src.slice(i, i + 12));
      if (m !== null) {
        const close = `"${m[1] as string}`;
        const start = i + m[0].length;
        const end = src.indexOf(close, start);
        i = end < 0 ? src.length : end + close.length;
        continue;
      }
    }
    if (c === '"') {
      let j = i + 1;
      let val = "";
      while (j < src.length) {
        if (src[j] === "\\") {
          val += src[j + 1] ?? "";
          j += 2;
          continue;
        }
        if (src[j] === '"') break;
        val += src[j];
        j += 1;
      }
      toks.push({ kind: "str", value: val, depth });
      i = j + 1;
      continue;
    }
    if (c === "'") {
      // 字符字面量（'\n' / '\u{…}'）vs 生命周期（'a）——两者都不是判定面
      if (src[i + 1] === "\\") {
        const endq = src.indexOf("'", i + 2);
        i = endq < 0 ? i + 2 : endq + 1;
        continue;
      }
      if (src[i + 2] === "'") {
        i += 3;
        continue;
      }
      i += 1;
      while (i < src.length && isIdentPart(src[i] as string)) i += 1;
      continue;
    }
    if (/[0-9]/.test(c)) {
      while (i < src.length && /[0-9a-zA-Z_.]/.test(src[i] as string)) i += 1;
      continue;
    }
    if (isIdentStart(c)) {
      let j = i;
      while (j < src.length && isIdentPart(src[j] as string)) j += 1;
      toks.push({ kind: "ident", value: src.slice(i, j), depth });
      i = j;
      continue;
    }
    if (c === "(" || c === "[" || c === "{") {
      toks.push({ kind: "punct", value: c, depth });
      depth += 1;
      i += 1;
      continue;
    }
    if (c === ")" || c === "]" || c === "}") {
      toks.push({ kind: "punct", value: c, depth });
      depth = Math.max(0, depth - 1);
      i += 1;
      continue;
    }
    if (/\s/.test(c)) {
      i += 1;
      continue;
    }
    toks.push({ kind: "punct", value: c, depth });
    i += 1;
  }
  return toks;
}

/** 路径规范化：折叠重复斜杠、去尾斜杠、`{name:regex}` → `{name}`（OAS 模板合法形）。 */
export function normalizeRoutePath(p: string): string {
  let out = p.replace(/\{([^}:]+):[^}]*\}/g, "{$1}").replace(/\/{2,}/g, "/");
  if (out.length > 1 && out.endsWith("/")) out = out.slice(0, -1);
  return out === "/" ? "" : out;
}

/** 作用域前缀 + 字面量拼接（前缀 `''` = 服务根）。 */
function joinRoutePath(prefix: string, lit: string): string {
  if (lit === "") return normalizeRoutePath(prefix);
  if (lit.startsWith("/")) return normalizeRoutePath(`${prefix}${lit}`);
  return normalizeRoutePath(prefix === "" ? `/${lit}` : `${prefix}/${lit}`);
}

/** 判定 `(` 之前的 callee 是否为 `crud_routes` / `crud_routes_with_refs`（泛型参数已跳过）。 */
function isCrudCallee(toks: RtTok[], parenIndex: number): boolean {
  let genericDepth = 0;
  for (let k = parenIndex - 1; k >= 0 && parenIndex - k <= 80; k -= 1) {
    const t = toks[k] as RtTok;
    if (t.kind === "punct") {
      if (t.value === ">") genericDepth += 1;
      else if (t.value === "<") genericDepth -= 1;
      else if (t.value === ")" || t.value === "]" || t.value === "}")
        return false;
      else if (t.value === ";" || t.value === "=") return false;
      continue;
    }
    if (
      t.kind === "ident" &&
      (t.value === "crud_routes" || t.value === "crud_routes_with_refs")
    ) {
      return genericDepth === 0;
    }
  }
  return false;
}

/**
 * 结构化路由抽取（见本节头注）：产出**相对 service 根**的实有路径 + 绑定方法。
 * 同一路径多次注册 ⇒ 方法合并。
 */
export function extractRouteEntries(src: string): RouteEntry[] {
  const toks = tokenizeRust(src);
  const acc = new Map<string, Set<string>>();
  const scopes: { prefix: string; base: number }[] = [];
  const entries: { path: string; methods: Set<string>; base: number }[] = [];
  const curPrefix = (): string =>
    scopes.length > 0 ? scopes[scopes.length - 1]!.prefix : "";
  const record = (path: string, methods: Iterable<string>): void => {
    const key = normalizeRoutePath(path);
    const set = acc.get(key) ?? new Set<string>();
    for (const m of methods) set.add(m);
    if (set.size > 0) acc.set(key, set);
  };
  const openEntry = (path: string, base: number): void => {
    entries.push({ path, methods: new Set<string>(), base });
  };
  const bindMethod = (name: string, depth: number): void => {
    for (let k = entries.length - 1; k >= 0; k -= 1) {
      const e = entries[k] as { base: number; methods: Set<string> };
      if (e.base <= depth) {
        e.methods.add(name);
        return;
      }
    }
  };
  const closeTo = (after: number, statementEnd: boolean): void => {
    while (entries.length > 0) {
      const top = entries[entries.length - 1]!;
      if (statementEnd ? top.base >= after : top.base > after) {
        const done = entries.pop()!;
        record(done.path, done.methods);
      } else break;
    }
    while (scopes.length > 0) {
      const top = scopes[scopes.length - 1]!;
      if (statementEnd ? top.base >= after : top.base > after) scopes.pop();
      else break;
    }
  };
  const ident = (t: RtTok | undefined, v: string): boolean =>
    t?.kind === "ident" && t.value === v;
  const colon = (t: RtTok | undefined): boolean =>
    t?.kind === "punct" && t.value === ":";

  for (let i = 0; i < toks.length; i += 1) {
    const t = toks[i]!;
    if (t.kind === "punct") {
      if (t.value === ";") {
        closeTo(t.depth, true);
        continue;
      }
      if (t.value === ")" || t.value === "]" || t.value === "}") {
        closeTo(t.depth - 1, false);
        continue;
      }
      if (t.value !== "(") continue;
    } else continue;

    const prev = toks[i - 1];
    const p2 = toks[i - 2];
    const p3 = toks[i - 3];
    const p4 = toks[i - 4];
    const arg = toks[i + 1];

    // web::scope("S")：前缀入栈（含 `/service/{id}` 挂载段——文档路径 = /api + 注册路径全形）
    if (ident(prev, "scope") && colon(p2) && colon(p3) && ident(p4, "web")) {
      if (arg?.kind === "str") {
        scopes.push({
          prefix: joinRoutePath(curPrefix(), arg.value),
          base: t.depth,
        });
      }
      continue;
    }
    // web::resource("R")：条目开启（其后 `.route(handler)` 绑方法）
    if (ident(prev, "resource") && colon(p2) && colon(p3) && ident(p4, "web")) {
      if (arg?.kind === "str")
        openEntry(joinRoutePath(curPrefix(), arg.value), t.depth);
      continue;
    }
    // web::<method>()：绑定到最内层仍开放的条目
    if (
      prev?.kind === "ident" &&
      ROUTE_METHOD_NAMES.has(prev.value) &&
      colon(p2) &&
      colon(p3) &&
      ident(p4, "web")
    ) {
      bindMethod(prev.value, t.depth);
      continue;
    }
    // `.route(…)` / `cfg.route(…)`：字符串形 = 路径条目；handler 形 = Resource 方法绑定
    if (ident(prev, "route") && p2?.kind === "punct" && p2.value === ".") {
      if (arg?.kind === "str")
        openEntry(joinRoutePath(curPrefix(), arg.value), t.depth);
      else if (entries.length === 0) openEntry(curPrefix(), t.depth);
      continue;
    }
    // crud_routes::<…>("S")：语法糖展开（同 crud crate 实现）
    if (isCrudCallee(toks, i) && arg?.kind === "str") {
      const base = joinRoutePath(curPrefix(), arg.value);
      record(base, ["get", "post"]);
      record(`${base}/{id}`, ["get", "put", "delete"]);
      record(`${base}/batch`, ["delete"]);
      continue;
    }
  }
  // 语句未以 `;` 收尾的尾部条目（如文件末尾）
  closeTo(-1, false);
  return [...acc.entries()].map(([path, methods]) => ({
    path,
    methods: [...methods],
  }));
}

/// 递归收集目录下全部 .rs 源码（字符串拼接）。
function collectSourceTree(root: string): string {
  if (!existsSync(root)) return "";
  let text = "";
  const walk = (dir: string): void => {
    for (const name of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, name.name);
      if (name.isDirectory()) walk(p);
      else if (name.name.endsWith(".rs")) text += readFileSync(p, "utf-8");
    }
  };
  walk(root);
  return text;
}

/// 路由模板归一：`{...}` 参数段 → `{}`（比对用）。参数**命名**差异（`{id}` vs `{waybill_id}`）
/// 不是路由漂移（同一 actix 路由）；真漂移 = 段数或字面段不一致。
function normalizeRouteTemplate(route: string): string {
  return route.replace(/\{[^}]*\}/g, "{}");
}

function crossCheckActionPaths(
  root: string,
  ns: string,
  serviceId: string,
  items: ActionItem[],
): string[] {
  const routes = collectHandlerRoutes(root, ns, serviceId);
  if (routes.size === 0) return []; // 无 handler 源码（如纯契约 service）→ 跳过
  const normalizedRoutes = new Set([...routes].map(normalizeRouteTemplate));
  const mismatches: string[] = [];
  for (const a of items) {
    if (a.direction === "outbound") continue;
    const rel: string = a.path || "";
    if (rel === "") {
      // 服务根动作：handler 注册 route("") / resource("")
      if (!routes.has("")) mismatches.push(`(root) route("") 缺失`);
      continue;
    }
    if (!normalizedRoutes.has(normalizeRouteTemplate(rel)))
      mismatches.push(rel);
  }
  return mismatches;
}

function pascalToKebab(s: string): string {
  // ContractMatter → contract-matter
  return s
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1-$2")
    .replace(/_/g, "-")
    .toLowerCase();
}

/// 动作 scope 动词（对齐 `pep/middleware.rs::method_to_action` 委托的
/// `ngac_contract::action_for_method`：GET=read / POST=create / PUT|PATCH=update /
/// DELETE=delete）。未知动词 MUST NOT 缺省——PEP 对未知动词要求不可授予的
/// `access:{rt}`，文档若缺省 `create` 即与执行分歧（review M70 r1 P3）⇒ fail-closed。
const SCOPE_VERB_BY_METHOD: Record<string, string> = {
  get: "read",
  post: "create",
  put: "update",
  patch: "update",
  delete: "delete",
};

/// PEP 映射面动词（单一来源 = 上表键集）：其余动词（head/options/trace/connect）
/// 不生成文档契约——PEP 对其要求不可授予的 `access:{rt}`。
const PEP_METHOD_VERBS = new Set(Object.keys(SCOPE_VERB_BY_METHOD));

/// 端点所需 scope（PEP 同规则推导，2026-10-07 口径修正）：`{verb}:{实体}`，实体由
/// **最终文档路径**派生（`scopeEntityFromDocPath`，与
/// `common::openapi_scope::service_entity_from_path` 同形；如 `/quality/evaluations` → `quality`，
/// 服务根 ⇒ `service_{svc}`）——CRUD / 动作 / 兜底端点共用同一推导（禁第二份）。
/// 未知动词 fail-closed（PEP 对未知动词要求不可授予的 `access:{rt}`）。
function scopeForDocPath(docPath: string, method: string): string {
  const verb = SCOPE_VERB_BY_METHOD[method];
  if (!verb) {
    throw new OpenapiDocError(
      `动词不在 PEP 映射面（get/post/put/patch/delete）: ${method}` +
        `（PEP 对未知动词要求 access:${scopeEntityFromDocPath(docPath)}，不可经签发面授予——拒绝生成文档）`,
    );
  }
  return `${verb}:${scopeEntityFromDocPath(docPath)}`;
}

/// provider 派发键形状（`[a-z0-9-]+`）——与 `outbound_admin.rs::is_slug`、适配器目录代号、
/// `instantiate-ns-host` 生成的 feature 名同形：形状漂移即跨面失配，故在生成期 fail-closed。
/// 规约：`openspec/specs/openapi-external-access :: oas3-outbound-contract-provider-binding`。
const PROVIDER_SLUG_RE = /^[a-z0-9-]+$/;

/// 动作型端点（非 CRUD）→ OAS3 path。声明源：`Pre-Proc/{ns}/openapi-actions.json`
/// （格式见 WZ openapi-actions.json 头注）。
/// 每个动作按 PEP 同规则推导 scope（见 `scopeForDocPath`；清单 `scope` 字段已退役——
/// 禁第二份推导）；`direction: "outbound"` 的动作（WZ → 第三方）不生成平台 paths，
/// 归入返回值 `outbound` 供文档 `x-outbound-contracts` 段；根路径 `''` = 服务根。
function buildActionPaths(
  serviceId: string,
  actions: ActionItem[],
): { paths: Record<string, unknown>; outbound: ActionItem[] } {
  const paths: Record<string, unknown> = {};
  const outbound: ActionItem[] = [];
  for (const a of actions || []) {
    if (a.direction === "outbound") {
      // provider 必填（fail-closed）：无 provider 的出向契约无法与任何适配器面互判，
      // 是一份无人可实现的声明 —— 宁可生成失败，不产出幽灵契约。
      const provider = (a.provider ?? "").trim();
      if (!PROVIDER_SLUG_RE.test(provider)) {
        throw new OpenapiDocError(
          `openapi-actions: ${serviceId} 出向条目 MUST 声明 slug 形态的 provider` +
            `（派发键 = Ext-adapter/<ext> 目录代号）: ` +
            `${a.operationId || a.path || "(unnamed)"} 实得 ${JSON.stringify(a.provider ?? null)}`,
        );
      }
      outbound.push({ ...a, provider });
      continue;
    }
    const method = (a.method || "post").toLowerCase();
    const rel = a.path || "";
    if (rel !== "" && !rel.startsWith("/")) {
      throw new OpenapiDocError(
        `openapi-actions: ${serviceId} 动作路径必须以 / 开头: ${rel}`,
      );
    }
    const full =
      rel === ""
        ? `/api/service/${serviceId}`
        : `/api/service/${serviceId}${rel}`;
    const scope = scopeForDocPath(full, method);
    const responses: Record<string, unknown> = {
      "200": { description: "OK" },
      "400": { description: "Validation error" },
      "401": { description: "Unauthorized" },
      "403": { description: "Forbidden (NGAC deny or SCOPE_INSUFFICIENT)" },
      "404": { description: "Not found" },
      "429": { description: "Rate limited / quota exceeded" },
    };
    const op: Record<string, unknown> = {
      summary: a.summary || `${a.operationId || rel || "(root)"} (action)`,
      operationId:
        a.operationId ||
        `${method}-${(rel || "root").replace(/[^a-zA-Z0-9]+/g, "-")}`,
      security: [{ ssoOAuth2: [scope] }],
      responses,
    };
    if (a.parameters?.length) op.parameters = a.parameters;
    if (a.requestBody) {
      op.requestBody = {
        required: true,
        content: { "application/json": { schema: a.requestBody } },
      };
    }
    paths[full] = { [method]: op };
  }
  return { paths, outbound };
}

function buildPaths(
  root: string,
  serviceId: string,
  entities: Record<string, unknown>[],
  ns: string,
): {
  paths: Record<string, unknown>;
  schemas: Record<string, unknown>;
  routes: RouteEntry[];
} {
  const paths: Record<string, unknown> = {};
  const schemas: Record<string, unknown> = {};
  // handler 实有路由（**结构化**：作用域感知）——实体 CRUD 判定 + 兜底路径生成的真相源。
  // key = 挂载相对路径（实体段判定），value = 文档路径（`/api` + 注册路径全形）。
  const routes = collectHandlerRouteEntries(root, ns, serviceId);
  const routePaths = new Map<string, string>();
  for (const r of routes) {
    routePaths.set(mountRelative(r.path), docPathFor(serviceId, r.path));
  }
  for (const ent of entities) {
    const name: string = typeof ent.name === "string" ? ent.name : "";
    if (!name) continue;
    // exposeCrud === false：handler 为自定义端点（根 POST + /list + /{id} 等），
    // 实体不暴露标准 CRUD 五路径（动作已在 openapi-actions.json 声明）→ 跳过。
    if (ent.exposeCrud === false) continue;
    // routeSegment：显式资源路径段（handler web::resource 段），缺省 kebab(实体名)。
    const hasRouteSegment = typeof ent.routeSegment === "string";
    const segment: string = hasRouteSegment
      ? (ent.routeSegment as string)
      : pascalToKebab(name);
    // 交叉校验：**结构化实有路由**必须含该实体的集合端点（挂载相对 `/seg`；
    // 作用域感知——`web::scope("/seg")` + 内层 `.route("")` 形态同样命中；
    // change `fix-openapi-doc-coverage`）。
    let crudBase: string | undefined;
    if (routePaths.size > 0) {
      crudBase = routePaths.get(`/${segment}`);
      if (crudBase === undefined && !hasRouteSegment) {
        // 复数 fallback：crud_routes 常用复数资源段（stock-ledgers / ownership-records），
        // 实体名单数（stock_ledger / ownership_record）无显式 routeSegment 时自动匹配。
        const pluralCandidates = [
          `${segment}s`,
          `${segment}es`,
          segment.endsWith("y") ? `${segment.slice(0, -1)}ies` : "",
        ].filter(Boolean);
        for (const cand of pluralCandidates) {
          const hit = routePaths.get(`/${cand}`);
          if (hit !== undefined) {
            crudBase = hit;
            (ent as Record<string, unknown>).routeSegment = cand;
            break;
          }
        }
      }
      if (crudBase === undefined) {
        if (hasRouteSegment) {
          // 显式声明必须正确（fail-closed）
          throw new OpenapiDocError(
            `[openapi] 交叉校验失败: ${serviceId} 实体 "${name}" routeSegment "${segment}" 与 handler 注册不符`,
          );
        }
        // 无显式声明且 kebab 无集合端点 → 该实体无标准 CRUD handler，
        // 跳过 CRUD 路径生成（实有自定义端点由下方兜底路径覆盖；杜绝幽灵路径）
        console.warn(
          `[openapi] 跳过实体 CRUD: ${serviceId} "${name}"（结构化路由无 "/${segment}" 集合端点；如确有 CRUD 请补 routeSegment）`,
        );
        continue;
      }
    }
    const entity = (ent.routeSegment as string) || segment;
    const base = crudBase ?? `/api/service/${serviceId}/${entity}`;
    const schemaRef = `#/components/schemas/${name}`;
    // scope 由**最终文档路径**派生（PEP 同规则）——挂载段与 serviceId 不同名时
    // （实测 CT `/service/bin-locations`）不能由实体名直接拼。
    const scopes = {
      read: scopeForDocPath(base, "get"),
      create: scopeForDocPath(base, "post"),
      update: scopeForDocPath(base, "put"),
      delete: scopeForDocPath(base, "delete"),
    };

    // 列表 + 创建（集合端点）
    paths[base] = {
      get: {
        summary: `List ${name}`,
        operationId: `list-${entity}`,
        security: [{ ssoOAuth2: [scopes.read] }],
        parameters: [
          {
            name: "page",
            in: "query",
            schema: { type: "integer", default: 1 },
          },
          {
            name: "page_size",
            in: "query",
            schema: { type: "integer", default: 20, maximum: 100 },
          },
          { name: "sort", in: "query", schema: { type: "string" } },
          { name: "search", in: "query", schema: { type: "string" } },
        ],
        responses: {
          "200": {
            description: "OK",
            content: {
              "application/json": {
                schema: {
                  type: "object",
                  properties: {
                    data: { type: "array", items: { $ref: schemaRef } },
                    total: { type: "integer" },
                    page: { type: "integer" },
                    page_size: { type: "integer" },
                  },
                },
              },
            },
          },
          "401": { description: "Unauthorized" },
          "403": { description: "Forbidden (NGAC deny or SCOPE_INSUFFICIENT)" },
          "429": { description: "Rate limited / quota exceeded" },
        },
      },
      post: {
        summary: `Create ${name}`,
        operationId: `create-${entity}`,
        security: [{ ssoOAuth2: [scopes.create] }],
        requestBody: {
          required: true,
          content: { "application/json": { schema: { $ref: schemaRef } } },
        },
        responses: {
          "201": { description: "Created" },
          "400": { description: "Validation error" },
          "401": { description: "Unauthorized" },
          "403": { description: "Forbidden (NGAC deny or SCOPE_INSUFFICIENT)" },
          "429": { description: "Rate limited / quota exceeded" },
        },
      },
    };

    // 单条端点
    const item = `${base}/{id}`;
    paths[item] = {
      get: {
        summary: `Get ${name} by id`,
        operationId: `get-${entity}`,
        security: [{ ssoOAuth2: [scopes.read] }],
        parameters: [
          {
            name: "id",
            in: "path",
            required: true,
            schema: { type: "integer" },
          },
        ],
        responses: {
          "200": {
            description: "OK",
            content: { "application/json": { schema: { $ref: schemaRef } } },
          },
          "401": { description: "Unauthorized" },
          "403": { description: "Forbidden (NGAC deny or SCOPE_INSUFFICIENT)" },
          "404": { description: "Not found" },
          "429": { description: "Rate limited / quota exceeded" },
        },
      },
      put: {
        summary: `Update ${name}`,
        operationId: `update-${entity}`,
        security: [{ ssoOAuth2: [scopes.update] }],
        parameters: [
          {
            name: "id",
            in: "path",
            required: true,
            schema: { type: "integer" },
          },
        ],
        requestBody: {
          required: true,
          content: { "application/json": { schema: { $ref: schemaRef } } },
        },
        responses: {
          "200": { description: "OK" },
          "400": { description: "Validation error" },
          "401": { description: "Unauthorized" },
          "403": { description: "Forbidden (NGAC deny or SCOPE_INSUFFICIENT)" },
          "404": { description: "Not found" },
          "429": { description: "Rate limited / quota exceeded" },
        },
      },
      delete: {
        summary: `Delete ${name}`,
        operationId: `delete-${entity}`,
        security: [{ ssoOAuth2: [scopes.delete] }],
        parameters: [
          {
            name: "id",
            in: "path",
            required: true,
            schema: { type: "integer" },
          },
        ],
        responses: {
          "204": { description: "No content" },
          "401": { description: "Unauthorized" },
          "403": { description: "Forbidden (NGAC deny or SCOPE_INSUFFICIENT)" },
          "404": { description: "Not found" },
          "429": { description: "Rate limited / quota exceeded" },
        },
      },
    };
  }
  return { paths, schemas, routes };
}

export function generate(
  namespace: string,
  opts: GenerateOptions = {},
): OpenapiDoc {
  const root = opts.root ?? REPO_ROOT;
  const servicesDir = gatewaySourcesKindDir(root, namespace, "Services");
  let serviceDirs: string[] = [];
  try {
    serviceDirs = readdirSync(servicesDir, { withFileTypes: true })
      .filter((d) => d.isDirectory())
      .map((d) => d.name);
  } catch {
    throw new OpenapiDocError(
      `[openapi] services dir not found: ${servicesDir}`,
    );
  }

  const paths: Record<string, unknown> = {};
  const schemas: Record<string, unknown> = {};
  const tags: Set<string> = new Set();
  // 逐 service 的实有路由（CRUD/动作覆盖不到者 → 兜底落文档，见下方 AUTO_PATHS 段）
  const serviceRoutes: {
    serviceId: string;
    domain: string;
    routes: RouteEntry[];
  }[] = [];
  for (const sid of serviceDirs.sort()) {
    const sf = join(servicesDir, sid, "service.json");
    let meta: {
      ontology?: { entities?: Record<string, unknown>[] };
      domain?: string;
    };
    try {
      meta = JSON.parse(readFileSync(sf, "utf-8")) as typeof meta;
    } catch {
      continue; // 无 service.json 的目录跳过
    }
    const entities = meta.ontology?.entities || [];
    const domain = meta.domain || sid;
    tags.add(domain);
    const built = buildPaths(root, sid, entities, namespace);
    Object.assign(paths, built.paths);
    Object.assign(schemas, built.schemas);
    serviceRoutes.push({ serviceId: sid, domain, routes: built.routes });
  }

  // 动作型端点（openapi-actions.json，跨 service 清单）
  const actionFile = join(root, "Pre-Proc", namespace, "openapi-actions.json");
  const outboundContracts: ActionItem[] = [];
  try {
    const actionsDoc = JSON.parse(readFileSync(actionFile, "utf-8")) as {
      actions?: ActionEntry[];
    };
    for (const entry of actionsDoc.actions || []) {
      // 交叉校验（fail-closed）：inbound 动作路径必须出现在 handler 源码
      const mismatches = crossCheckActionPaths(
        root,
        namespace,
        entry.service,
        entry.items || [],
      );
      if (mismatches.length > 0) {
        throw new OpenapiDocError(
          `[openapi] 交叉校验失败: ${entry.service} 动作路径与 handler 注册不符: ${mismatches.join(", ")}`,
        );
      }
      const built = buildActionPaths(entry.service, entry.items || []);
      Object.assign(paths, built.paths);
      outboundContracts.push(...built.outbound);
    }
  } catch (e) {
    if (e instanceof OpenapiDocError) throw e;
    if ((e as NodeJS.ErrnoException).code !== "ENOENT") {
      const msg = e instanceof Error ? e.message : String(e);
      throw new OpenapiDocError(
        `[openapi] actions manifest error: ${actionFile}: ${msg}`,
      );
    }
    // 无 manifest → 跳过（仅 CRUD 文档）
  }

  // ── 实有路由兜底（change `fix-openapi-doc-coverage`）───────────────────────
  // CRUD 标准端点与 `openapi-actions.json` 声明之外，**其余实有路由**一律落文档
  // （§1.2「全量开放候选」口径；MUST NOT 静默跳过）。文档路径 = `/api` + 注册路径全形
  // （`docPathFor`，挂载段原样保留——实测 CT `/service/bin-locations`）；scope 按 PEP
  // 同规则由文档路径派生（`scopeForDocPath`，由 conformance 测试对账）；非 PEP 映射面
  // 动词（head/options/trace/connect）**不生成**——PEP 对其要求不可授予的 `access:{rt}`，
  // 生成即制造不可用契约。已被 CRUD/动作声明的 path×method 不覆盖。
  for (const { serviceId, domain, routes } of serviceRoutes) {
    for (const route of routes) {
      const rel = normalizeRoutePath(route.path);
      const full = docPathFor(serviceId, rel);
      const ops =
        (paths[full] as Record<string, Record<string, unknown>> | undefined) ??
        {};
      for (const method of route.methods) {
        if (!PEP_METHOD_VERBS.has(method) || ops[method] !== undefined) {
          continue;
        }
        const slug = (rel === "" ? "root" : rel.replace(/[^a-zA-Z0-9]+/g, "-"))
          .replace(/^-+|-+$/g, "")
          .toLowerCase();
        ops[method] = {
          summary: `${method.toUpperCase()} ${rel === "" ? "(service root)" : rel}`,
          operationId: `${method}-${serviceId}-${slug}`,
          tags: [domain],
          security: [{ ssoOAuth2: [scopeForDocPath(full, method)] }],
          responses: {
            "200": { description: "OK" },
            "400": { description: "Validation error" },
            "401": { description: "Unauthorized" },
            "403": {
              description: "Forbidden (NGAC deny or SCOPE_INSUFFICIENT)",
            },
            "404": { description: "Not found" },
            "429": { description: "Rate limited / quota exceeded" },
          },
        };
      }
      if (Object.keys(ops).length > 0) paths[full] = ops;
    }
  }
  return {
    openapi: "3.0.3",
    info: {
      title: `${namespace} OpenAPI`,
      version: "1.0.0",
      description:
        "AliothStudio 开放 API（openapi-external-access）。认证：先经 POST /auth/token " +
        "(client_credentials) 或 POST /auth/authenticate (API Key) 获取 Bearer JWT，" +
        "再携带 Authorization: Bearer <token> 调用。每个端点标注所需 scope（read/create/update/delete:{entity}），" +
        "scope 不足返回 403 SCOPE_INSUFFICIENT；限流超限返回 429。",
    },
    servers: [{ url: "/" }],
    tags: [...tags].map((t) => ({ name: t })),
    paths,
    ...(outboundContracts.length > 0
      ? { "x-outbound-contracts": outboundContracts }
      : {}),
    components: {
      schemas,
      securitySchemes: {
        ssoOAuth2: {
          type: "oauth2",
          flows: {
            clientCredentials: {
              tokenUrl: "/api/auth/token",
              scopes: {},
            },
          },
        },
      },
    },
  };
}
