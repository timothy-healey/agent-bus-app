# `--json-schema` under `--print --output-format stream-json`

Research for issue #7, which blocks "Spec: structured verdicts" (#8). Map: #3.

Grounded in five real runs of the installed CLI, `claude` **2.1.289**, with
`--model claude-opus-5-5`, from a throwaway temp dir. The excerpts below are
trimmed but otherwise verbatim. Session IDs, message/tool IDs, UUIDs, timestamps
and rate-limit/billing fields have been removed, and the temp dir is shown as
`$TMP`.

## Answer

- **Mechanism.** `--json-schema` adds a synthetic tool called **`StructuredOutput`**
  (it shows up in `system/init.tools`), and the schema becomes that tool's input
  schema. The model answers by *calling the tool*. The CLI validates the input
  against the schema and returns a `tool_result`: either
  `"Structured output provided successfully"` or, when validation fails, an
  `is_error: true` result listing the schema violations.
- **Where it appears.** The validated object is in the final `result` event as
  **`structured_output`**, a parsed JSON object. When the run succeeds, the `result`
  field holds the same object as a JSON *string*. The same object also appears
  earlier, as `input` on the `StructuredOutput` `tool_use` block inside an
  `assistant` event. The final assistant turn may contain no `text` block at all.
- **Agentic runs.** It works. In a run that used tools, the model called `Read`
  first and called `StructuredOutput` last. `structured_output` was filled in the
  same way.
- **Failure has no error subtype.** When the model cannot satisfy the schema, the
  CLI sends one `[structured-output-enforce]` user nudge. The model then stops with
  prose, and the run ends with **`subtype: "success"`, `is_error: false`, and the
  `structured_output` key absent**. `result` holds the prose. In this run no
  `error_*` subtype appeared and the exit code was 0. The only reliable failure
  signal is **the absence of `structured_output`**.
- **`--append-system-prompt`.** Both flags can be passed together. When the
  appended prompt asked for a trailing `VERDICT:` line, the model ignored it. It
  wrote prose and then called `StructuredOutput`, so the text contained no
  `VERDICT:` line. The schema wins over a text convention.
- **Hooks.** `StructuredOutput` is an ordinary tool as far as hooks are
  concerned. `PreToolUse` and `PostToolUse` hooks with matcher `"*"` (loaded with
  `--settings`) fired for it, with `tool_name: "StructuredOutput"` and the verdict
  object as `tool_input`. A hook that blocks tools could therefore block the
  verdict. `--allowedTools=Read` did not stop `StructuredOutput` from being used.

## Evidence

All runs used this wrapper, with `cd $TMP` first:

```sh
claude --print --output-format stream-json --verbose --model claude-opus-5-5 \
  --json-schema "$(cat <schema>.json)" [extra flags] "<prompt>"
```

Verdict schema (`verdict.json`), used in runs 1, 2, 4 and 5:

```json
{"type":"object","properties":{"verdict":{"type":"string","enum":["approve","revise","reject"]},"reason":{"type":"string"}},"required":["verdict","reason"],"additionalProperties":false}
```

### Run 1: no tools

Prompt: `"Is 2+2=4? Approve if true."` Exit 0. Event types: `system/init`,
`system/hook_started`, `system/hook_response`, `system/commands_changed`,
`rate_limit_event`, 1 `assistant`, 1 `user`, `result/success`.

`system/init` (trimmed). `StructuredOutput` was added to the tool list:

```json
{"type":"system","subtype":"init","tools":["Task","Bash","Edit","Read", "...", "StructuredOutput", "...","Write"],"model":"claude-opus-5-5","permissionMode":"default"}
```

The assistant turn is a single `tool_use`. There is no `text` block:

```json
{"type":"assistant","message":{"model":"claude-opus-5-5","role":"assistant","content":[{"type":"tool_use","name":"StructuredOutput","input":{"verdict":"approve","reason":"2+2 does equal 4, so the statement is true."},"caller":{"type":"direct"}}],"stop_reason":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":25549,"cache_read_input_tokens":0,"output_tokens":24}}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Structured output provided successfully"}]},"tool_use_result":"Structured output provided successfully"}
```

`result`, trimmed. Usage blocks and IDs are removed:

```json
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"stop_reason":"tool_use","terminal_reason":"completed","api_error_status":null,
 "result":"{\"verdict\":\"approve\",\"reason\":\"2+2 does equal 4, so the statement is true.\"}",
 "structured_output":{"verdict":"approve","reason":"2+2 does equal 4, so the statement is true."}}
```

On success, `stop_reason` is `"tool_use"`, not `"end_turn"`.

### Run 2: uses a tool before answering

Setup: `report.txt` containing `Status: all tests pass.` / `Blocker: none.`.
Flags: `--allowedTools=Read`. Prompt: `"Read report.txt in the current directory
with the Read tool, then approve if it has no blocker, otherwise revise."`
Exit 0.

(`--allowedTools Read`, with a space, is variadic and swallows the positional
prompt. The CLI then exits 1 with `Input must be provided either through stdin or
as a prompt argument`. Use the `=` form, or put the prompt before it.)

The content blocks, in order:

```text
assistant  tool_use     Read              {"file_path":"$TMP/report.txt"}
user       tool_result                    "1\tStatus: all tests pass.\n2\tBlocker: none.\n3\t"
assistant  tool_use     StructuredOutput  {"verdict":"approve","reason":"report.txt says \"Status: all tests pass.\" and \"Blocker: none.\" It lists no blocker."}
user       tool_result                    "Structured output provided successfully"
```

```json
{"type":"result","subtype":"success","is_error":false,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"completed",
 "result":"{\"verdict\":\"approve\",\"reason\":\"report.txt says \\\"Status: all tests pass.\\\" and \\\"Blocker: none.\\\" It lists no blocker.\"}",
 "structured_output":{"verdict":"approve","reason":"report.txt says \"Status: all tests pass.\" and \"Blocker: none.\" It lists no blocker."}}
```

### Run 3: unsatisfiable schema

Schema (`impossible.json`): integer `n` with `minimum: 10, maximum: 5`. Flags:
`--max-turns=6`, a spend cap that the run never reached. Prompt:
`"Give me the number 7."` **Exit 0.**

The content blocks, in order. Thinking blocks are elided:

```text
assistant  tool_use     StructuredOutput  {"n":7}
user       tool_result  is_error:true     "Output does not match required schema: /n: must be <= 5, /n: must be >= 10"
assistant  text         "**7**\n\nI couldn't return it through the structured output tool. That tool's schema requires `n` to be at least 10 and at most 5, so no number can pass. ..."
user       text         "[structured-output-enforce] You MUST call the StructuredOutput tool to complete this request. Call this tool now."
assistant  tool_use     StructuredOutput  {"n":7}
user       tool_result  is_error:true     "Output does not match required schema: /n: must be <= 5, /n: must be >= 10"
assistant  text         "The number is **7**, but StructuredOutput rejected it again with the same error. ... No value can satisfy that schema, so retrying won't help. ..."
```

```json
{"type":"result","subtype":"success","is_error":false,"num_turns":4,"stop_reason":"end_turn","terminal_reason":"completed","api_error_status":null,
 "result":"The number is **7**, but StructuredOutput rejected it again with the same error. ..."}
```

`jq 'has("structured_output")'` on that `result` event returns `false`. So
validation failure looks like this:

- The model sees each failure as an `is_error` `tool_result`, with JSON-pointer
  messages, and can retry.
- The CLI injects **one** `[structured-output-enforce]` user message after the
  model gives up in prose.
- If the model still fails, the run ends as an ordinary **`success`** with prose in
  `result` and no `structured_output`. In this run no dedicated error subtype was
  emitted. A different failure path, such as many retries, might produce one, but
  this run did not show it.

### Run 4: `--append-system-prompt` together with the schema

Flags: `--append-system-prompt="You are a REVIEWER. End your reply with a line:
VERDICT: approve | revise | reject"`. Prompt: `"Is 2+2=5? Review it."` Exit 0.

```text
assistant  text         "No. 2 + 2 = 4, not 5. ... The claim should be rejected. The fix is: 2 + 2 = 4."   (no VERDICT: line)
assistant  tool_use     StructuredOutput  {"verdict":"reject","reason":"2 + 2 = 4 in standard arithmetic (Peano axioms). ..."}
user       tool_result                    "Structured output provided successfully"
```

```json
{"type":"result","subtype":"success","is_error":false,"num_turns":2,
 "result":"{\"verdict\":\"reject\",\"reason\":\"2 + 2 = 4 in standard arithmetic (Peano axioms). ...\"}",
 "structured_output":{"verdict":"reject","reason":"2 + 2 = 4 in standard arithmetic (Peano axioms). ..."}}
```

The appended prompt changed nothing about the mechanism. The model produced prose
*and* a structured verdict, but it dropped the text-line convention the appended
prompt asked for.

### Run 5: hooks

Flags: `--settings=$TMP/hooks.json`, with `PreToolUse` and `PostToolUse` hooks
(matcher `"*"`) that append their stdin to a log. Prompt: `"Is 3+3=6? Approve if
true."` Exit 0. Hook payloads, trimmed:

```json
{"hook_event_name":"PreToolUse","tool_name":"StructuredOutput","tool_input":{"verdict":"approve","reason":"3 + 3 = 6 is true."}}
{"hook_event_name":"PostToolUse","tool_name":"StructuredOutput","tool_response":"Structured output provided successfully"}
```

`result.structured_output` was `{"verdict":"approve","reason":"3 + 3 = 6 is true."}`.
The user's own `SessionStart` hook also ran in every run, as the
`system/hook_started` and `system/hook_response` events. It had no effect on
structured output.

### Cost note

Each trivial run cost about $0.21–0.32 at list price (`total_cost_usd`), almost all
of it from about 25.5k tokens of cache creation for the system prompt and tools.
Adding `StructuredOutput` adds one tool definition and one extra turn
(`num_turns` 2 instead of 1).

## What this means for today's parser

`src-tauri/runners/src/stream_json.rs` collects every assistant `text` block into
`self.text`, and falls back to `result` (as a string) only when there is no text.
Then `parse_verdict` / `parse_items` scan for `VERDICT:` / `ARTIFACT:` / `KEY:`
lines. Under `--json-schema`, that pipeline breaks without any error:

- Run 1 or run 2 shape (no text block): `self.text` falls back to the JSON string
  in `result`. It has no `VERDICT:` line, so `parse_verdict` returns the default
  **`Revise`**, even though the model approved.
- Run 4 shape (prose plus tool call): `self.text` is the prose, which has no
  `VERDICT:` line. The result is again a default **`Revise`**.
- The `"result"` arm never reads `structured_output`. The rate-limit check is not
  affected, because a `rate_limit_event` with `status: "allowed"` is neither
  `type: "error"` nor `is_error: true`.

## Implications for "Spec: structured verdicts" (#8)

1. **Read `result.structured_output` as the source of truth.** It is already parsed
   and already validated by the CLI. Do not parse `result` (a string that could be
   either JSON or prose) or assistant text. The `StructuredOutput` `tool_use` input
   could feed a live display, but the final event is authoritative.
2. **Absence means failure.** A `result` event with `subtype: "success"` but no
   `structured_output` is a contract failure, such as an unsatisfiable or
   contradictory schema, and must be handled explicitly (Revise, Retry or Error,
   as #8 decides). Do not assume that `is_error`, the subtype or the exit code will
   flag it.
3. **One schema per role replaces `output_contract`.** The prose contract that
   `output_contract()` appends to the system prompt (`KEY:` / `ARTIFACT:` /
   `VERDICT:` lines) can become a JSON Schema. For example, a reviewer could return
   `{items:[{key, artifact, verdict, reason}]}` and a producer could return
   `{items:[{key, artifact, description}]}`. Keep the schemas satisfiable and
   simple: an enum for verdict and plain strings for paths. Validation errors go
   back to the model, but the CLI only nudges once before it gives up.
4. **The `--append-system-prompt` text should describe the task, not the format.**
   Once a schema is passed, a text convention in the system prompt is ignored, so
   any leftover `VERDICT:` instruction is dead weight that can mislead.
5. **Scope enforcement and hooks must allow `StructuredOutput`.** Any `PreToolUse`
   hook or permission rule written for "real scope enforcement" must let
   `StructuredOutput` through. If it blocks that tool, every verdict becomes a
   "no `structured_output`" failure. `--allowedTools` does not block it.
   `--disallowedTools` and `--tools` were not tested.
6. **Argv.** `build_args` gains `--json-schema <schema-json>`. Pass it as one argv
   element, the same way `--append-system-prompt` is passed today. Variadic flags
   such as `--allowedTools` need the `=` form or must not sit directly before the
   positional prompt.
7. **Usage accounting does not change.** `result.usage` stays authoritative. The
   extra `StructuredOutput` turn is already included in it.
