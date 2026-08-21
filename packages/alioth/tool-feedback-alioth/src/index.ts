/**
 * Model-facing feedback consumer tools — the agent side of the page-
 * annotation loop: list open annotations, acknowledge, resolve (with the
 * fix summary), or dismiss. Deterministic over `ctx.aliothFeedback`
 * (feedback-alioth capability); the browser overlay and HTTP carrier live in
 * feedback-web-alioth.
 * @module @dsh-alioth/tool-feedback-alioth
 */

import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import z from '@deepseek-ai/schemastery'

export const name = 'tool-feedback-alioth'
export const inject = ['tools', 'aliothFeedback']

export interface Config {}

export const Config: z<Config> = z.object({})

/** Per-tool inline output (schema + render) — mirrors the harness canonical
 * todo_write shape; keeping it inline preserves the schema→value inference. */
const statusOutput = (status: string, id: string, reply: string | undefined): Array<{ type: 'text'; text: string }> => [
  { type: 'text', text: `${status} ${id}${reply === undefined ? '' : ` — ${reply}`}` },
]

export function apply(ctx: Context, _config: Config): void {
  const svc = () => ctx.aliothFeedback

  ctx.tools.register(defineTool({
    name: 'alioth_feedback_pending',
    description: '列出所有未处理的页面批注（pending/acknowledged，新→旧）：comment、url、elementPath、cssClasses。进入批注消费循环前先调用它。',
    parameters: {},
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          count: { type: 'number', required: true },
          annotations: {
            type: 'array',
            required: true,
            items: {
              type: 'object',
              additionalProperties: false,
              properties: {
                id: { type: 'string', required: true },
                status: { type: 'string', required: true },
                url: { type: 'string' },
                comment: { type: 'string' },
                element: { type: 'string' },
                elementPath: { type: 'string' },
                cssClasses: { type: 'string' },
                reply: { type: 'string' },
              },
            },
          },
        },
      },
      render: (_args: {}, value: { count: number; annotations: Array<{ id: string; status: string; comment: string; url: string }> }) => [
        { type: 'text', text: `${value.count} 条待处理批注` },
        ...value.annotations.map(a => ({ type: 'text' as const, text: `#${a.id} [${a.status}] ${a.comment} @ ${a.url}` })),
      ],
    },
    async execute() {
      const annotations = svc().pending().map(a => ({
        id: a.id,
        status: a.status,
        url: a.url,
        comment: a.comment,
        element: a.element,
        elementPath: a.elementPath,
        cssClasses: a.cssClasses,
        ...(a.reply === null ? {} : { reply: a.reply }),
      }))
      return { annotations, count: annotations.length }
    },
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_feedback_ack',
    description: '认领一条批注：标记 acknowledged 并附处理中说明，防止其他消费方重复认领。',
    parameters: {
      id: { type: 'string', required: true, description: '批注 id' },
      reply: { type: 'string', description: '处理中说明' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          id: { type: 'string', required: true },
          status: { type: 'string', required: true },
          reply: { type: 'string' },
        },
      },
      render: (_args: { id: string }, value: { id: string; status: string; reply?: string }) => statusOutput(value.status, value.id, value.reply),
    },
    async execute(args) {
      const annotation = svc().setStatus(args.id, 'acknowledged', args.reply ?? '处理中')
      return { id: annotation.id, status: annotation.status, ...(annotation.reply === null ? {} : { reply: annotation.reply }) }
    },
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_feedback_resolve',
    description: '结案一条批注：必须携带修复要点 reply；终态后不可再流转。',
    parameters: {
      id: { type: 'string', required: true, description: '批注 id' },
      reply: { type: 'string', required: true, description: '已修复的要点说明（必填）' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          id: { type: 'string', required: true },
          status: { type: 'string', required: true },
          reply: { type: 'string' },
        },
      },
      render: (_args: { id: string }, value: { id: string; status: string; reply?: string }) => statusOutput(value.status, value.id, value.reply),
    },
    async execute(args) {
      const annotation = svc().setStatus(args.id, 'resolved', args.reply)
      return { id: annotation.id, status: annotation.status, ...(annotation.reply === null ? {} : { reply: annotation.reply }) }
    },
  }))

  ctx.tools.register(defineTool({
    name: 'alioth_feedback_dismiss',
    description: '不处理并结案一条批注（附原因）；终态后不可再流转。',
    parameters: {
      id: { type: 'string', required: true, description: '批注 id' },
      reply: { type: 'string', required: true, description: '不处理原因（必填）' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          id: { type: 'string', required: true },
          status: { type: 'string', required: true },
          reply: { type: 'string' },
        },
      },
      render: (_args: { id: string }, value: { id: string; status: string; reply?: string }) => statusOutput(value.status, value.id, value.reply),
    },
    async execute(args) {
      const annotation = svc().setStatus(args.id, 'dismissed', args.reply)
      return { id: annotation.id, status: annotation.status, ...(annotation.reply === null ? {} : { reply: annotation.reply }) }
    },
  }))

  ctx.logger.info('tool-feedback-alioth: 4 feedback consumer tools registered')
}
