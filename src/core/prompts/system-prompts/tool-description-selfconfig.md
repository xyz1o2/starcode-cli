<!--
name: 'Tool Description: SelfConfig'
description: Read and change your own runtime settings — context compaction, tool output budget and tool call timeouts. `inspect` shows current values, their source and your remaining change budget; `set` changes one; `reset` undoes changes.
-->
Read and change the runtime settings that govern how you work. This replaces telling the user
to go set an environment variable and restart.

**Actions**: `inspect` | `set` | `reset`

**Params**

| param | required | notes |
| --- | --- | --- |
| `action` | yes | one of `inspect`, `set`, `reset` |
| `key` | for `set` | knob key exactly as listed by `inspect`, e.g. `compact.max_tokens` |
| `value` | for `set` | string; type and range are per-knob and listed by `inspect` |
| `reason` | for `set` | what you observed that makes this worth changing now |
| `persist` | no | write to `~/.star/self-tuning.json` so it survives restarts; default `false` |

## When to use this

Reach for it when you hit a condition you can name, not as routine:

- context is repeatedly getting compacted, or is close to the compaction threshold
- a tool result was truncated and you need more of it
- a tool call after an approval prompt timed out

Then: `inspect` first, so the change is based on the real current value rather than a guess.

## When not to use this

- **Do not tune to feel faster.** You have no measurement of "faster"; you would just be
  spending a limited budget on noise.
- **Do not tune to avoid thinking about a hard problem.** Loosening the compaction
  threshold to keep more context around is a real trade — you may overflow the model.
- **Do not tune mid-task just because the last turn felt awkward.** That is not evidence.
- Prefer telling the user what you observed over spending budget on a speculative change.

## What you cannot change here

Model and context window are frozen into the per-message agent when a turn starts, so they
cannot be changed from inside a running turn. `inspect` lists them as read-only and points at
`/model` and `/context`. Settings captured at startup (for example the request timeout baked
into the HTTP client) are not listed at all — they are genuinely not changeable at runtime,
and pretending otherwise would waste your budget.

## Three limits, and why they exist

- **Change budget.** A few successful changes per session. An agent that discovers it can
  flip settings tends to flip them every turn, which ends up worse than not tuning at all.
  The budget is reported by `inspect` *before* you spend it.
- **Operator locks.** A knob an operator pinned cannot be changed. `inspect` marks these.
- **`reason` is mandatory.** It is recorded so a change can be told apart from a guess.

`reset` never consumes budget and never runs out, so you can always undo a change that turns
out to be wrong. If you are out of budget, reset is still available — and if the situation
genuinely needs a different configuration, say so to the user instead of tuning further.