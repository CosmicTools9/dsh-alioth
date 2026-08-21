/**
 * `@dsh-alioth/feedback-alioth` — the visual feedback CAPABILITY (page
 * annotations for frontend debugging), ported from AliothStudio's
 * `scripts/feedback` dev tool: a persistent annotation store with the
 * consumption state machine and a long-poll watch seam. No HTTP here —
 * `feedback-web-alioth` is the carrier; `tool-feedback-alioth` is the
 * model-facing consumer.
 *
 * State machine (identical to the AliothStudio original):
 *   pending ⇄ acknowledged; both → resolved | dismissed (terminal);
 *   same-status PATCH is idempotent; reply-only PATCH keeps the status.
 *
 * Storage: `node:sqlite` (no dependency), default `~/.dsh-alioth/feedback.db`,
 * `Config.dbPath` overrides. Dev-tool trust boundary: loopback consumers,
 * carrier enforces origin allowlists for browser writes.
 * @module @dsh-alioth/feedback-alioth
 */

import { randomUUID } from 'node:crypto'
import { mkdirSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, resolve } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'

export const name = 'feedback-alioth'
export const inject: readonly string[] = []

export const ANNOTATION_STATUSES = ['pending', 'acknowledged', 'resolved', 'dismissed'] as const
export type AnnotationStatus = (typeof ANNOTATION_STATUSES)[number]

/** Allowed transitions; terminal states have no exits. */
const STATUS_TRANSITIONS: Record<AnnotationStatus, readonly AnnotationStatus[]> = {
  pending: ['acknowledged', 'resolved', 'dismissed'],
  acknowledged: ['pending', 'resolved', 'dismissed'],
  resolved: [],
  dismissed: [],
}

export interface FeedbackSession {
  id: string
  origin: string
  url: string
  createdAt: number
}

/** The on-page annotation a human left on a page element. */
export interface Annotation {
  id: string
  sessionId: string
  origin: string
  url: string
  comment: string
  element: string
  elementPath: string
  cssClasses: string
  status: AnnotationStatus
  reply: string | null
  createdAt: number
  updatedAt: number
}

export interface AddAnnotationInput {
  sessionId?: string
  origin: string
  url: string
  comment: string
  element?: string
  elementPath?: string
  cssClasses?: string
}

export interface AliothFeedbackService {
  health(): { ok: boolean; annotations: number; pending: number }
  /** Idempotent session per (origin, url). */
  ensureSession(origin: string, url: string): FeedbackSession
  addAnnotation(input: AddAnnotationInput): Annotation
  /** Open annotations (pending + acknowledged), newest first. */
  pending(): Annotation[]
  get(id: string): Annotation | null
  /**
   * Transition an annotation (validated) and/or write a reply. Same-status
   * PATCH is idempotent; terminal states accept reply writes only.
   * @throws on illegal transitions.
   */
  setStatus(id: string, status: AnnotationStatus | undefined, reply: string | undefined): Annotation
  /** Long-poll for open annotations; resolves early on a new arrival. */
  watch(timeoutMs: number): Promise<Annotation[]>
  /** Drop resolved/dismissed annotations older than the horizon; returns the count. */
  prune(olderThanMs: number): number
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothFeedback: AliothFeedbackService
  }
}

interface Waiter {
  settled: boolean
  timer: ReturnType<typeof setTimeout>
  resolve: (batch: Annotation[]) => void
}

const DEFAULT_DB_PATH = (): string => resolve(homedir(), '.dsh-alioth', 'feedback.db')

export interface Config {
  /** SQLite file for annotation persistence. */
  dbPath?: string
}

export const Config: z<Config> = z.object({
  dbPath: z.string(),
})

/** Pure storage + state machine core (also the test surface). */
export function createFeedbackStore(dbPath: string): AliothFeedbackService {
  mkdirSync(dirname(dbPath), { recursive: true })
  const db = new DatabaseSync(dbPath)
  db.exec(`
    CREATE TABLE IF NOT EXISTS sessions (
      id TEXT PRIMARY KEY,
      origin TEXT NOT NULL,
      url TEXT NOT NULL,
      created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS annotations (
      id TEXT PRIMARY KEY,
      session_id TEXT NOT NULL REFERENCES sessions(id),
      origin TEXT NOT NULL,
      url TEXT NOT NULL,
      comment TEXT NOT NULL,
      element TEXT NOT NULL DEFAULT '',
      element_path TEXT NOT NULL DEFAULT '',
      css_classes TEXT NOT NULL DEFAULT '',
      status TEXT NOT NULL DEFAULT 'pending',
      reply TEXT,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );
  `)
  const insertSession = db.prepare('INSERT INTO sessions (id, origin, url, created_at) VALUES (?, ?, ?, ?)')
  const findSession = db.prepare('SELECT * FROM sessions WHERE origin = ? AND url = ? LIMIT 1')
  const insertAnnotation = db.prepare(
    'INSERT INTO annotations (id, session_id, origin, url, comment, element, element_path, css_classes, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
  const selectPending = db.prepare(
    "SELECT * FROM annotations WHERE status IN ('pending', 'acknowledged') ORDER BY created_at DESC")
  const selectById = db.prepare('SELECT * FROM annotations WHERE id = ? LIMIT 1')
  const updateStatus = db.prepare('UPDATE annotations SET status = ?, reply = ?, updated_at = ? WHERE id = ?')
  const countAll = db.prepare('SELECT count(*) AS n FROM annotations')
  const deleteStale = db.prepare(
    "DELETE FROM annotations WHERE status IN ('resolved', 'dismissed') AND updated_at <= ?")

  const waiters = new Set<Waiter>()

  const toAnnotation = (row: Record<string, unknown>): Annotation => ({
    id: row.id as string,
    sessionId: row.session_id as string,
    origin: row.origin as string,
    url: row.url as string,
    comment: row.comment as string,
    element: row.element as string,
    elementPath: row.element_path as string,
    cssClasses: row.css_classes as string,
    status: row.status as AnnotationStatus,
    reply: (row.reply as string | null) ?? null,
    createdAt: row.created_at as number,
    updatedAt: row.updated_at as number,
  })

  const wake = (): void => {
    const batch = selectPending.all().map(toAnnotation)
    for (const waiter of waiters) {
      if (waiter.settled) continue
      waiter.settled = true
      clearTimeout(waiter.timer)
      waiters.delete(waiter)
      waiter.resolve(batch)
    }
  }

  const service: AliothFeedbackService = {
    health() {
      return {
        ok: true,
        annotations: (countAll.get() as { n: number }).n,
        pending: selectPending.all().length,
      }
    },
    ensureSession(origin: string, url: string): FeedbackSession {
      const existing = findSession.get(origin, url) as Record<string, unknown> | undefined
      if (existing !== undefined) {
        return { id: existing.id as string, origin: existing.origin as string, url: existing.url as string, createdAt: existing.created_at as number }
      }
      const session = { id: randomUUID(), origin, url, createdAt: Date.now() }
      insertSession.run(session.id, session.origin, session.url, session.createdAt)
      return session
    },

    addAnnotation(input) {
      if (input.comment.trim() === '') {
        throw new Error('aliothFeedback.addAnnotation: comment must not be empty')
      }
      const session = input.sessionId !== undefined && input.sessionId !== ''
        ? { id: input.sessionId }
        : service.ensureSession(input.origin, input.url)
      const now = Date.now()
      const annotation: Annotation = {
        id: randomUUID(),
        sessionId: session.id,
        origin: input.origin,
        url: input.url,
        comment: input.comment.trim(),
        element: input.element ?? '',
        elementPath: input.elementPath ?? '',
        cssClasses: input.cssClasses ?? '',
        status: 'pending',
        reply: null,
        createdAt: now,
        updatedAt: now,
      }
      insertAnnotation.run(annotation.id, annotation.sessionId, annotation.origin, annotation.url,
        annotation.comment, annotation.element, annotation.elementPath, annotation.cssClasses,
        annotation.status, annotation.createdAt, annotation.updatedAt)
      wake()
      return annotation
    },

    pending() {
      return selectPending.all().map(toAnnotation)
    },

    get(id) {
      const row = selectById.get(id) as Record<string, unknown> | undefined
      return row === undefined ? null : toAnnotation(row)
    },

    setStatus(id, status, reply) {
      const current = service.get(id)
      if (current === null) {
        throw new Error(`aliothFeedback.setStatus: annotation ${id} not found`)
      }
      let nextStatus = current.status
      if (status !== undefined && status !== current.status) {
        if (!STATUS_TRANSITIONS[current.status].includes(status)) {
          throw new Error(`aliothFeedback.setStatus: ${current.status} → ${status} is not an allowed transition`)
        }
        nextStatus = status
      }
      updateStatus.run(nextStatus, reply ?? current.reply, Date.now(), id)
      const updated = service.get(id)
      if (updated === null) throw new Error('aliothFeedback.setStatus: update lost the row')
      return updated
    },

    watch(timeoutMs) {
      return new Promise<Annotation[]>(resolveWatch => {
        const waiter: Waiter = {
          settled: false,
          timer: setTimeout(() => {
            if (waiter.settled) return
            waiter.settled = true
            waiters.delete(waiter)
            resolveWatch(selectPending.all().map(toAnnotation))
          }, Math.max(0, Math.min(timeoutMs, 60_000))),
          resolve: resolveWatch,
        }
        waiters.add(waiter)
      })
    },

    prune(olderThanMs) {
      const result = deleteStale.run(Date.now() - olderThanMs)
      return Number(result.changes)
    },
  }

  return service
}

export function apply(ctx: Context, config: Config): void {
  const dbPath = config.dbPath ?? process.env.DSH_FEEDBACK_DB_PATH ?? DEFAULT_DB_PATH()
  ctx.provide('aliothFeedback', createFeedbackStore(dbPath))
  ctx.logger.info(`feedback-alioth: annotation store at ${dbPath}`)
}
