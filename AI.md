# AI

## Provider abstraction (spec §35)

`src-tauri/src/ai.rs` speaks the OpenAI-compatible chat-completions protocol. Configuration in
Settings → AI:

- **Base URL** — default `https://api.openai.com/v1`; OpenAI-compatible HTTPS endpoints using
  bearer authentication also work (for example, OpenRouter). Plain HTTP is accepted only for
  loopback servers such as a local Ollama-compatible endpoint. Native Azure OpenAI endpoints use a
  different URL/authentication contract and are not directly supported.
- **Classification model** — cheap + fast (default
  [`gpt-4o-mini`](https://developers.openai.com/api/docs/models/gpt-4o-mini)).
- **Coaching model** — for goal breakdown / morning coach / daily review (default
  [`gpt-5.6-luna`](https://developers.openai.com/api/docs/models/gpt-5.6-luna)) at `low`
  reasoning effort. Luna requests use `max_completion_tokens`; the inexpensive classifier remains
  on `gpt-4o-mini` and compatible providers keep the legacy request shape. Existing settings that
  still use either former stock value (`gpt-4o` or `gpt-4.1-mini`) are upgraded; custom model values
  are preserved.
- **API key** — stored in the OS credential store (Windows Credential Manager) via `keyring`.
  Never in SQLite, never logged, excluded from exports.

Adding another provider means implementing the same three functions (`classify_activity`,
`break_down_goal`, `coach`)
behind a different transport; nothing else in the app knows how classification happens.

## What AI is used for

1. **Activity classification (layer 3)** — only for activity that deterministic rules, your
   corrections, and the cache could not settle. Strict JSON contract:
   `{"classification": "focused|supporting|neutral|distracted", "confidence": 0-1, "reason": "…"}`
   — parsed and validated; anything else is rejected. `idle`/`unknown` may not be assigned by AI.
   Confidence below **0.65** is stored as *Unknown* and the user can settle it later (spec §12).
2. **Goal breakdown** — turns one interview outcome into a simple (3–4), standard (5–7), or
   detailed (8–10) action checklist. The goal is handled as untrusted data, the response must match
   a validated `{"steps":["…"]}` JSON contract, and every generated step remains editable.
3. **Morning coach (spec §24)** — pushback on today's plan grounded in your actual history
   (avg completed/day, estimation bias, completion by start hour). The numbers are computed
   deterministically and handed to the model; it may narrate them, not invent them.
4. **Daily review analysis (spec §22)** — ≤100 words, factual, no motivational filler
   (the system prompt forbids cheerleading).
5. **Insight narration (spec §23)** — rewrites deterministic pattern findings; numbers must be
   kept verbatim.

## Cost control (spec §36)

Order is always **rules → corrections → cache → AI**:

- Default blocked-domain rules and app rules settle the obvious cases for free.
- Every manual correction becomes a matcher that outranks AI forever after.
- AI answers are cached by `(commitment, process, domain, normalized title)` — the same document
  open all day costs one call, not hundreds.
- In-flight de-duplication guarantees one call per unique context even across live + stored paths.
- Privacy-setting changes and activity deletion invalidate delayed AI work so it cannot repopulate
  data after the boundary; delayed AI results also cannot overwrite a manual correction.
- No AI is ever called on raw polling events, idle, private apps, or when no commitment is active.

## Offline mode (spec §37)

With AI disabled or unreachable: tasks, planning, monitoring, deterministic + manual
classification, focus timers, check-ins, interventions, breaks, reviews and all scoring work
unchanged. Ambiguous sessions are stored as *Unknown* for one-click manual classification, and
insights fall back to their deterministic sentences.
