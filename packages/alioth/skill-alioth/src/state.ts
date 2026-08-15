/**
 * Track/step state machine over a parsed adapter. The model drives steps one
 * at a time; the machine tracks position and reports the current step's
 * instruction + gates. Gate execution and persistence are separate modules.
 * @module @dsh-alioth/skill-alioth/state
 */

import type { Adapter, Step, Track } from './adapter.ts'

/** Position within one track. */
export interface RunPosition {
  readonly trackIndex: number
  readonly stepIndex: number
}

export interface RunState {
  readonly adapter: Adapter
  readonly position: RunPosition
  /** Step ids completed in order. */
  readonly completed: readonly string[]
}

export interface RunTransition {
  readonly previous: RunPosition
  readonly next: RunPosition
  readonly finished: boolean
}

export function initialRunState(adapter: Adapter): RunState {
  return { adapter, position: { trackIndex: 0, stepIndex: 0 }, completed: [] }
}

/** The step at `position`, if any. */
export function currentStep(state: RunState): { track: Track; step: Step } | undefined {
  const track = state.adapter.tracks[state.position.trackIndex]
  const step = track?.steps[state.position.stepIndex]
  return track === undefined || step === undefined ? undefined : { track, step }
}

function nextPosition(state: RunState): RunPosition {
  const track = state.adapter.tracks[state.position.trackIndex]
  if (track === undefined) {
    return state.position
  }
  if (state.position.stepIndex + 1 < track.steps.length) {
    return { trackIndex: state.position.trackIndex, stepIndex: state.position.stepIndex + 1 }
  }
  return { trackIndex: state.position.trackIndex + 1, stepIndex: 0 }
}

/**
 * Mark the current step complete and advance. Idempotent completion guard:
 * completing a step already in `completed` is a no-op transition to the same
 * position (callers that re-run gates should not double-advance).
 */
export function completeCurrentStep(state: RunState): { state: RunState; transition: RunTransition } {
  const step = currentStep(state)
  const previous = state.position
  if (step === undefined) {
    return { state, transition: { previous, next: previous, finished: true } }
  }
  if (state.completed.includes(step.step.id)) {
    return { state, transition: { previous, next: previous, finished: false } }
  }
  const next = nextPosition(state)
  const finished = next.trackIndex >= state.adapter.tracks.length
  return {
    state: {
      adapter: state.adapter,
      position: next,
      completed: [...state.completed, step.step.id],
    },
    transition: { previous, next, finished },
  }
}
