#!/usr/bin/env bun
/**
 * appagent-client.ts — AppAgent 会话驱动客户端（Meta REST，202 受理 + 轮询契约）
 *
 * 复用于评测驱动（`appagent-build-eval.ts`）。驱动语义与 `scripts/e2e/appagent-dialog-gate.ts`
 * 同源（同一 202/polling 契约、同一信封解包、同一死代理规避）——**E2E 驱动尚未 import 本模块**
 * （它内联了同构 helper）；本模块是该面的可复用单一实现，E2E 驱动迁移列为后续任务，
 * 迁移前不得再写第三份（REUSE_FIRST：新实现已记录原因＝原实现未导出、不可 import）。
 *
 * 契约要点（META_AI_SPEC §3.1）：
 * - `POST /api/meta/chat-sessions/{id}/dialog` → 202 `{ user_message_id, status: "running" }`
 * - 回复经 `GET /api/meta/chat-sessions/{id}` 轮询消息取回（`id > user_message_id` 的首条 assistant）
 * - turn 终态由消息派生：running（user 后无 assistant）/ succeeded / failed（内容前缀「对话执行失败：」）
 * - LLM 未配置 → 503 `LLM_NOT_CONFIGURED`（**环境不可达**，调用方须判 degraded，不得判通过）
 */

// 死代理规避（本机 ALL_PROXY 指向未监听端口 → 本地请求假阴性）
for (const k of [
  'ALL_PROXY',
  'all_proxy',
  'HTTP_PROXY',
  'http_proxy',
  'HTTPS_PROXY',
  'https_proxy',
]) {
  delete process.env[k];
}
process.env.NO_PROXY = '127.0.0.1,localhost';

export const META_API_BASE = process.env.META_API_BASE ?? 'http://127.0.0.1:4949';

/** 默认 Meta dev 账号（与 scripts/e2e/appagent-dialog-gate.ts 一致） */
export const DEFAULT_USER = 'admin:admin123';

export type Rec = Record<string, unknown>;

export class HttpError extends Error {
  constructor(
    readonly status: number,
    readonly path: string,
    readonly body: string,
  ) {
    super(`${path} → HTTP ${status}: ${body.slice(0, 300)}`);
  }
}

/** 环境不可达（backend 缺席 / LLM 未配置 / 5xx）→ 调用方判 degraded，不得判通过 */
export class UnreachableError extends Error {}

export async function api(path: string, init: RequestInit = {}, token?: string): Promise<Rec> {
  const headers: Record<string, string> = {
    'content-type': 'application/json',
    ...((init.headers as Record<string, string>) ?? {}),
  };
  if (token) headers.authorization = `Bearer ${token}`;

  let res: Response;
  try {
    res = await fetch(`${META_API_BASE}${path}`, { ...init, headers });
  } catch (e) {
    throw new UnreachableError(`无法连接 ${META_API_BASE}${path}：${(e as Error).message}`);
  }
  const text = await res.text();
  if (!res.ok) {
    if (res.status === 503 || res.status >= 500) {
      throw new UnreachableError(`${path} → HTTP ${res.status}: ${text.slice(0, 200)}`);
    }
    throw new HttpError(res.status, path, text);
  }
  let body: Rec = {};
  try {
    body = text ? (JSON.parse(text) as Rec) : {};
  } catch {
    body = { raw: text };
  }
  // ApiResponse::success 包装 → 取 data（无包装时原样）
  return (body.data as Rec) ?? body;
}

export async function login(userPass = DEFAULT_USER): Promise<string> {
  const [username, password] = userPass.split(':');
  const r = await api('/api/meta/auth/login', {
    method: 'POST',
    body: JSON.stringify({ username, password }),
  });
  const token = r.access_token as string | undefined;
  if (!token) throw new UnreachableError(`登录未返回 access_token（凭据或服务异常）`);
  return token;
}

export async function createSession(
  token: string,
  namespace: string,
  title: string,
): Promise<string> {
  const r = await api(
    '/api/meta/chat-sessions',
    { method: 'POST', body: JSON.stringify({ title, namespace }) },
    token,
  );
  const id = (r.id ?? (r.session as Rec | undefined)?.id) as string | number | undefined;
  if (id === undefined) throw new Error(`建会话未返回 id: ${JSON.stringify(r).slice(0, 200)}`);
  return String(id);
}

/** 发送一轮 dialog，返回本轮 user 消息 id（游标） */
export async function sendDialog(token: string, sid: string, content: string): Promise<string> {
  const r = await api(
    `/api/meta/chat-sessions/${sid}/dialog`,
    { method: 'POST', body: JSON.stringify({ content }) },
    token,
  );
  const cur = (r.user_message_id ?? r.userMessageId) as string | number;
  return String(cur);
}

export type Msg = { id: string; role: string; content: string };

export async function messages(token: string, sid: string): Promise<Msg[]> {
  const r = await api(`/api/meta/chat-sessions/${sid}`, {}, token);
  const raw = (r.messages ?? []) as Rec[];
  return raw.map((m) => ({
    id: String(m.id),
    role: String(m.role),
    content: String(m.content ?? ''),
  }));
}

export type TurnOutcome = {
  /** succeeded | failed | aborted（超时视为 running 悬挂 = 中断待续，不计 succeeded） */
  status: 'succeeded' | 'failed' | 'timeout';
  reply: string;
  messageId: string | null;
  elapsedMs: number;
};

/** 轮询直到出现 id > afterId 的 assistant 消息（或超时） */
export async function waitReply(
  token: string,
  sid: string,
  afterId: string,
  timeoutMs: number,
): Promise<TurnOutcome> {
  const after = BigInt(afterId);
  const t0 = Date.now();
  const deadline = t0 + timeoutMs;
  while (Date.now() < deadline) {
    const msgs = await messages(token, sid);
    const hit = msgs.find((m) => m.role === 'assistant' && BigInt(m.id) > after);
    if (hit) {
      const failed = hit.content.startsWith('对话执行失败：');
      return {
        status: failed ? 'failed' : 'succeeded',
        reply: hit.content,
        messageId: hit.id,
        elapsedMs: Date.now() - t0,
      };
    }
    await new Promise((r) => setTimeout(r, 3000));
  }
  return { status: 'timeout', reply: '', messageId: null, elapsedMs: Date.now() - t0 };
}

/** 一轮完整对话：建会话 → 发消息 → 轮询回复 */
export async function runTurn(
  token: string,
  namespace: string,
  title: string,
  prompt: string,
  timeoutMs: number,
): Promise<{ sessionId: string; outcome: TurnOutcome }> {
  const sid = await createSession(token, namespace, title);
  const cursor = await sendDialog(token, sid, prompt);
  const outcome = await waitReply(token, sid, cursor, timeoutMs);
  return { sessionId: sid, outcome };
}
