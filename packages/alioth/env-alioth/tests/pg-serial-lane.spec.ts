/**
 * The SQL funnel's single-lane invariant.
 *
 * Every caller reaches PostgreSQL through one `PgHandle`, and that handle owns
 * exactly one `pg.Client`. Two statements issued concurrently on one client
 * overlap: pg 8 only warns ("Calling client.query() when the client is already
 * executing a query") while pg 9 rejects outright, and — worse than the noise —
 * the handle's replay predicates assume nothing else is in flight (a refused
 * statement may be replayed, one that merely failed never may). These cases pin
 * the lane directly and then through a real connection.
 * @module @dsh-alioth/env-alioth/tests/pg-serial-lane
 */

import { describe, expect, it } from 'vitest'
import { acquirePostgres, createSerialLane } from '../src/index.ts'
import { createTestDatabase } from './test-db.ts'

describe('createSerialLane', () => {
  it('runs one task at a time, in FIFO order', async () => {
    const lane = createSerialLane()
    const order: string[] = []
    let inFlight = 0
    let peak = 0
    const task = (name: string, ms: number) => async (): Promise<string> => {
      inFlight += 1
      peak = Math.max(peak, inFlight)
      order.push(`start:${name}`)
      const settled = Promise.withResolvers<void>()
      setTimeout(settled.resolve, ms)
      await settled.promise
      order.push(`end:${name}`)
      inFlight -= 1
      return name
    }

    const results = await Promise.all([
      lane(task('a', 30)),
      lane(task('b', 5)),
      lane(task('c', 1)),
    ])

    expect(peak).toBe(1)
    expect(order).toEqual(['start:a', 'end:a', 'start:b', 'end:b', 'start:c', 'end:c'])
    expect(results).toEqual(['a', 'b', 'c'])
  })

  it('keeps the lane alive across a rejection', async () => {
    const lane = createSerialLane()
    const failing = lane(async () => {
      throw new Error('boom')
    })
    await expect(failing).rejects.toThrow('boom')
    await expect(lane(async () => 'next')).resolves.toBe('next')
  })

  it('keeps every task’s own result', async () => {
    const lane = createSerialLane()
    const [one, two] = await Promise.all([
      lane(async () => 1),
      lane(async () => 'two'),
    ])
    expect(one).toBe(1)
    expect(two).toBe('two')
  })
})

describe('PgHandle single-lane queries', () => {
  it('serializes concurrent statements on its one connection', async () => {
    const db = await createTestDatabase('pglane')
    const handle = await acquirePostgres({ url: db.url })
    try {
      const sleepMs = 120
      const started = Date.now()
      const results = await Promise.all([
        handle.query<{ n: number }>(`SELECT 1 AS n FROM (SELECT pg_sleep(${sleepMs / 1000})) AS slept`),
        handle.query<{ n: number }>(`SELECT 1 AS n FROM (SELECT pg_sleep(${sleepMs / 1000})) AS slept`),
        handle.query<{ n: number }>(`SELECT 1 AS n FROM (SELECT pg_sleep(${sleepMs / 1000})) AS slept`),
      ])
      const elapsed = Date.now() - started

      // Overlapped statements would finish in ~one sleep; serialized ones take
      // all three. The bound only has to exclude the overlap, so a slow machine
      // cannot fail it.
      expect(elapsed).toBeGreaterThanOrEqual(sleepMs * 2)
      expect(results.map(result => result.rows[0]?.n)).toEqual([1, 1, 1])
    } finally {
      await handle.close()
      await db.dispose()
    }
  }, 60_000)
})
