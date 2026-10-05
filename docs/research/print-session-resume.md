# `--resume`, `--session-id` and `--fork-session` under `--print`

Research for issue #12 (map #3, unblocks spec #13 "session resume for revise loops").
Grounded in real runs of the installed CLI, `claude --version` → `2.1.289 (Claude Code)`, subscription auth, model `haiku` (`claude-haiku-4-5-20251001`) to keep plan usage small. All runs were made from a throwaway `mktemp -d` directory, called `$T` below.

## Answer

**Yes.** A separate `claude --print --output-format stream-json --verbose` process can `--resume <session_id>` a worker's session, and the prompt cache carries over. In the revise test, the resumed reviser wrote **3,430** cache tokens against **20,277** for a fresh reviser given the same task. It also skipped re-reading the files.

`--fork-session` gives the reviser its own new session id and leaves the original session file byte-identical. `--session-id <uuid>` lets the app pre-assign an id, and it works together with `--resume … --fork-session`.

Two caveats change the design:

1. **A different `--append-system-prompt` on resume is silently ignored.** `--system-prompt-snapshot` defaults to `on`: the first run's system prompt (appended text included) is recorded and replayed on every resume. `--system-prompt-snapshot off` makes the new prompt apply, but it invalidates the cache, so the run costs about as much as a fresh one. To keep the savings, put reviser instructions in the **user message**.
2. **A different `--settings` on resume does apply.** A denied tool disappears from the tool list. That changes the cached prefix, so the cache misses: 23,745 cache tokens written on the first request, against about 450 with the same settings.

Resume does not depend on cwd. A session written under one directory was resumed from another, and the fork was stored under the new cwd's project directory.

## Token comparison

All figures come from the `result` event's `usage` (summed over the run's API requests). "First request" is the first assistant message's `usage`, before any tools run.

| Run | What | API requests | input | cache_creation | cache_read | output | First request: creation / read |
|---|---|---|---|---|---|---|---|
| A | fresh writer: Read context.md, Write summary.md | 3 | 26 | 19,868 | 70,843 | 701 | 8,945 / 14,172 |
| B | `--resume A`, new append prompt, "revise to 5 bullets" | 2 | 18 | **3,430** | 68,526 | 561 | **446 / 34,040** |
| F | fresh run, same reviser prompt and task (must Read both files) | 3 | 26 | **20,277** | 71,020 | 940 | 8,950 / 14,172 |
| C | `--resume A --fork-session`, short question | 1 | 10 | 447 | 37,470 | 155 | 447 / 37,470 |
| C-off | same as C plus `--system-prompt-snapshot off` | 1 | 10 | 23,735 | 14,172 | 78 | 23,735 / 14,172 |
| E1 | fork A from a **different cwd** | 1 | 10 | 573 | 37,470 | 71 | 573 / 37,470 |
| E2 | fork A with a **different `--settings`** (Write/Edit denied) | 5 | 42 | 27,666 | 171,700 | 1,015 | 23,745 / 13,455 |
| G | `--resume A --fork-session --session-id <uuid-G>` | 1 | 10 | 436 | 37,470 | 86 | 436 / 37,470 |

How to read these numbers:

- About 14.2k tokens of the CLI's own system prompt and tools are cached across *all* sessions. Every fresh run reads them (F's first request: 14,172 read), so "fresh" never means fully uncached.
- In a fresh run, everything after that shared prefix has to be written to the cache: the appended prompt, CLAUDE.md and memory context, the file reads. A resumed run reads the whole prior conversation from the cache (B's first request: 34,040 read, 446 written).
- B against F, weighted by API list-price multipliers (cache write 1.25×, cache read 0.1× of input, 5-minute TTL): B ≈ 18 + 4,288 + 6,853 ≈ **11.2k input-equivalent tokens**; F ≈ 26 + 25,346 + 7,102 ≈ **32.5k**. That is about **65% less**. How cache tokens count toward *subscription* Utilization is not published, so treat this as a relative guide, not a quota figure.
- B also needed one fewer API request, because the files were already in context and it did not re-Read them.
- The context file was about 11.7k characters. Random-word text tokenizes poorly, so the read took about 10k tokens instead of the planned 2–3k. The relative comparison is unaffected.
- Not tested: cache TTL expiry. All resumes ran within minutes of A. A reviser started after the cache TTL lapses would pay the full creation cost, but would still save the file re-reads.

## Evidence

The base flags below are the same as the worker argv in `src-tauri/runners/src/command.rs`, minus `--add-dir` and `--max-thinking-tokens`:

```
BASE="--print --output-format stream-json --verbose --model haiku \
      --settings $T/settingsA.json --permission-mode acceptEdits"
# settingsA.json: {"permissions":{"allow":["Write","Read","Edit"]}}
# settingsB.json: {"permissions":{"allow":["Read"],"deny":["Write","Edit"]}}
```

### (a) Fresh writer, then (b) resume with a different `--append-system-prompt`

```
cd $T/wt1
claude $BASE --append-system-prompt "You are the WRITER worker. End every reply with the exact tag [W]." \
  "Read context.md. Write summary.md with a 3-bullet summary of it that includes the document codeword. Then reply with one short sentence."
# init   session_id=<session-A> cwd=$T/wt1
# result session_id=<session-A> num_turns=3
# text:  "Summary created with the HERON-42 codeword included.  [W]"

claude $BASE --append-system-prompt "You are the REVISER worker. End every reply with the exact tag [R]." \
  --resume <session-A> \
  "Revise summary.md: make it 5 bullets and keep the codeword. Then reply with one short sentence."
# init   session_id=<session-A>          <- same id, appended to the same session
# tools: Edit only (no Read: the file was already in context)
# text:  "Expanded to 5 bullets with HERON-42 still included.  [W]"   <- WRITER tag; new append prompt ignored
```

Fresh baseline F: same reviser `--append-system-prompt` and task, run in a fresh directory that holds A's `summary.md` and `context.md`. It made two Reads and an Edit, and replied `"... [R]"`.

The session record at `~/.claude/projects/<encoded-cwd>/<session-A>.jsonl` contains the WRITER prompt text. This matches `claude --help`:

> `--system-prompt-snapshot <on|off>` Record the system prompt once per conversation and reuse it verbatim on every request and resume. on (the default): the prompt is rendered on the conversation's first request — a --system-prompt or --append-system-prompt included — sent, and recorded; every later request and resume sends the record as-is, even when a later launch passes different text, until the conversation is compacted. off: never record; the prompt is rendered fresh every request.

### (c) `--resume` + `--fork-session`

```
shasum ~/.claude/projects/<encoded-wt1>/<session-A>.jsonl   # c9c43ca5…  (58 lines)

claude $BASE --append-system-prompt "...REVISER... [R]." --resume <session-A> --fork-session \
  "Without using tools: what is the document codeword, and how many bullets does summary.md have now? One sentence."
# init   session_id=<session-C>          <- new id
# text:  "The document codeword is HERON-42 and summary.md now has 5 bullets.  [W]"
#        (it remembers B's revision, so the fork copies the full history up to now)

# with --system-prompt-snapshot off added:
# init   session_id=<session-C-off>
# text:  "The document codeword is HERON-42.  [R]"   <- new prompt applies
# usage: cache_creation 23,735 / cache_read 14,172  <- whole conversation re-cached

shasum ~/.claude/projects/<encoded-wt1>/<session-A>.jsonl   # c9c43ca5…  unchanged
```

The hash was rechecked after all five forks (C, C-off, E1, E2, G). It was still unchanged, and each fork wrote its own `<session-X>.jsonl`.

### (d) `--session-id <uuid>`

```
claude --print --output-format stream-json --verbose --model haiku --session-id <uuid-D> "Reply with the single word OK."
# init/result session_id=<uuid-D>      <- pre-assigned id honoured

# same command again:
# exit 1, stderr: "Error: Session ID <uuid-D> is already in use."

claude ... --resume <uuid-D> "What single word did you reply with last time? Answer in one word."
# session_id=<uuid-D>, text "OK", cache_read 23,068 / creation 114

claude $BASE --resume <session-A> --fork-session --session-id <uuid-G> "Without using tools: what is the document codeword? One sentence."
# init/result session_id=<uuid-G>      <- the fork takes the pre-assigned id
```

### (e) Different cwd and different `--settings`

```
cd $T/wt2      # a different directory holding a copy of context.md
claude $BASE --resume <session-A> --fork-session "Without using tools: what is the document codeword? One sentence."
# init cwd=$T/wt2  session_id=<session-E1>   text "The document codeword is HERON-42.  [W]"
# cache_read 37,470 (full hit)
# new session stored under ~/.claude/projects/<encoded-wt2>/, not wt1's directory
```

A follow-up fork from `wt2` asked the model to state its system prompt's working directory and then run `pwd`. Both answers were `$T/wt2`, and the snapshotted `[W]` tag still applied. So the environment's cwd follows the new launch, while the appended prompt text stays frozen. The history still holds the absolute `wt1` paths from A's tool calls.

```
cd $T/wt1
claude --print ... --settings $T/settingsB.json --permission-mode acceptEdits \
  --resume <session-A> --fork-session \
  "Append the line 'reviewed' to summary.md using the Edit or Write tool. Then reply with one short sentence."
# init tools list: Edit and Write ABSENT (present in A/B's init)
# Edit  -> "<tool_use_error>Error: No such tool available: Edit. Edit is disabled for this session…"
# Write -> "<tool_use_error>Error: No such tool available: Write. Write is disabled for this session…"
# Bash  `echo "reviewed" >> …/summary.md`  -> succeeded; permission_denials: []
# first request: cache_creation 23,745 / cache_read 13,455  <- prefix changed, cache miss
```

The new settings scope does apply on resume. The model then used Bash to get around the denied tools. This run did not establish why Bash ran without a prompt (user-level allow rules or `acceptEdits` behaviour). That belongs to the scope-enforcement ticket, not here.

## Implications for "Spec: session resume for revise loops" (#13)

1. **Resume the writer's session with `--fork-session`; never resume it in place.** A fork keeps the writer's session as an immutable record. Several revisers or reviewers can branch from the same point, and a failed revise can be retried from a clean base. In-place `--resume` (run B) appends to the writer's session and changes what the next resume sees.
2. **Pre-assign ids with `--session-id`.** The app can generate a UUID per invocation and pass `--session-id` on fresh runs *and* on forks (run G). That way it knows the id before the process starts, without parsing it out of `init`/`result`. A reused id fails fast (`already in use`, exit 1), so retries must mint a new UUID.
3. **Reviser instructions go in the user message, not `--append-system-prompt`.** Under the default snapshot, a resumed run keeps the *writer's* appended system prompt. `--system-prompt-snapshot off` fixes the role but loses the cache, which makes resuming barely worth it. The spec should decide between the two options. A user-message role ("You are now reviewing… ") keeps the savings.
4. **Keep `--settings` identical to the writer's if you want the cache.** A reviser with a narrower scope (for example, no Write) gets that scope enforced, but pays roughly fresh cost. Where the reviser needs a different scope, the spec can accept that cost, or treat resume as worth it only when the scope matches.
5. **Run the reviser in the same worktree.** Resume works across cwd, but the forked history holds the original worktree's absolute paths, and the model will act on them. If the revise runs in a different worktree, the spec should either prefer a fresh run or tell the model in the user message which tree to use.
6. **Keep the lost-session fallback.** The LLM Chat ACL already handles a dead `--resume` session by retrying fresh once (`chat_with_retry`, D6 in `src-tauri/llm_chat/src/claude_cli.rs`). The worker revise path needs the same fallback: the session file may be gone, compacted or expired from the cache. Session ids should stay inside the runner adapter, as F3 does for chat.
7. **The savings depend on how fast the revise follows the write.** The cache-read win assumes the revise starts within the cache TTL. After that, a resume still saves the file re-reads and keeps the context, but the cache creation cost comes back.

## Sources

- `claude --help` for 2.1.289: `--resume`, `--session-id`, `--fork-session`, `--system-prompt-snapshot`, `--no-session-persistence`.
- Live runs above (stream-json `system/init`, `assistant.message.usage`, `result.usage`, `result.permission_denials`).
- Session records under `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` (read only, used for hashes and to confirm the appended prompt was recorded).
- Prompt-cache price multipliers (1.25× write and 0.1× read for the 5-minute TTL): Anthropic prompt caching docs, https://docs.claude.com/en/docs/build-with-claude/prompt-caching
