# Can a subscription `claude` CLI list available models and effort levels?

Research for issue #5 (blocks #6, "Spec: Effort levels + model/effort pickers"). Checked against the installed `claude` 2.1.289, signed in with a claude.ai subscription (OAuth, `apiKeySource: "none"`, no `ANTHROPIC_API_KEY` set).

## Answer

Yes, it can, but not through a subcommand or the `system/init` event. The SDK `initialize` control request, sent over `--input-format stream-json`, returns a `models` array. Each entry gives the alias or ID to pass to `--model`, the resolved model ID, a display name, and, where the model supports effort, `supportsEffort: true` plus `supportedEffortLevels` (for example, Opus 4.6 and Sonnet 4.6 list no `xhigh`, and Haiku 4.5 has no effort fields at all). The request makes no model call, and the process exits once stdin closes, so the pickers can be filled by query. The CLI does not enforce effort itself. An unknown `--effort` value only prints a warning to stderr and exits 0. A level the model does not support, or effort on a model with no effort support, is accepted silently, and the `init` event never reports the effort in use. The app therefore has to validate effort against `supportedEffortLevels`. An unavailable model fails fast and recognisably: exit 1, an `assistant` message with `error: "model_not_found"`, a `result` with `is_error: true` and `api_error_status: 404`, and $0 cost.

## Evidence

Every probe ran from a throwaway temp directory with `--no-session-persistence`, using one-line prompts on the cheapest model the question allowed. Outputs are trimmed, and IDs, emails, org names, paths and the PID are removed.

### 1. No subcommand or flag lists models

`claude --help` (2.1.289) lists the commands `agents, attach, auth, auto-mode, doctor, gateway, import, install, logs, mcp, plugin, purge, respawn, rm, setup-token, stop, ultrareview, update`. None of them lists models. The flags say:

```
--effort <level>   Effort level for the current session (low, medium, high, xhigh, max)
--model <model>    Model for the current session. Provide an alias for the latest model
                   (e.g. 'fable', 'opus', or 'sonnet') or a model's full name.
```

`claude auth status` reports `"authMethod": "claude.ai"` and `"subscriptionType"`, and nothing about models.

### 2. The `initialize` control request returns models with effort levels

```sh
echo '{"type":"control_request","request_id":"r1","request":{"subtype":"initialize"}}' > init.in
claude -p --input-format stream-json --output-format stream-json --verbose \
  --no-session-persistence < init.in > init.jsonl
```

Exit 0. The stream holds only `system/hook_started`, `system/hook_response` and one `control_response`. It has no `init`, `assistant` or `result` event, so no model was called.

`.response.response | keys`:

```
["account","agents","analytics_disabled","available_output_styles","capabilities","commands",
 "current_permission_mode","fast_mode_disabled_reason","fast_mode_state","feedback_mode",
 "ide_rc_auto_enable_gate","models","output_style","pid","remote_control_auto_connect_default",
 "remote_control_auto_enable","remote_control_auto_on_by_default","remote_control_available",
 "session_state","user_output_styles_dir"]
```

`.response.response.models` had 12 entries. Selected entries, verbatim:

```json
{ "value": "default", "resolvedModel": "claude-opus-5-5", "displayName": "Default (recommended)",
  "description": "Opus 5.5 · Best for everyday, complex tasks", "supportsEffort": true,
  "supportedEffortLevels": ["low","medium","high","xhigh","max"],
  "supportsAdaptiveThinking": true, "supportsFastMode": true, "supportsAutoMode": true }
{ "value": "sonnet", "resolvedModel": "claude-sonnet-5-5", "displayName": "Sonnet 5.5",
  "description": "Most efficient for simpler tasks", "supportsEffort": true,
  "supportedEffortLevels": ["low","medium","high","xhigh","max"],
  "supportsAdaptiveThinking": true, "supportsAutoMode": true }
{ "value": "haiku", "resolvedModel": "claude-haiku-4-5-20251001", "displayName": "Haiku 4.5",
  "description": "Fastest for quick answers" }
{ "value": "claude-sonnet-4-6", "resolvedModel": "claude-sonnet-4-6", "displayName": "Sonnet 4.6",
  "description": "Efficient for routine tasks", "supportsEffort": true,
  "supportedEffortLevels": ["low","medium","high","max"],
  "supportsAdaptiveThinking": true, "supportsAutoMode": true }
```

All 12 `value`s, with their effort levels:

| value | resolvedModel | effort levels |
|---|---|---|
| default | claude-opus-5-5 | low, medium, high, xhigh, max |
| opus | claude-opus-5-5 | low, medium, high, xhigh, max |
| fable | claude-fable-5-1 | low, medium, high, xhigh, max |
| sonnet | claude-sonnet-5-5 | low, medium, high, xhigh, max |
| haiku | claude-haiku-4-5-20251001 | (none: no `supportsEffort`) |
| claude-sonnet-5 | claude-sonnet-5 | low, medium, high, xhigh, max |
| claude-opus-5 | claude-opus-5 | low, medium, high, xhigh, max |
| claude-fable-5 | claude-fable-5 | low, medium, high, xhigh, max |
| claude-opus-4-8 | claude-opus-4-8 | low, medium, high, xhigh, max |
| claude-opus-4-7 | claude-opus-4-7 | low, medium, high, xhigh, max |
| claude-opus-4-6 | claude-opus-4-6 | low, medium, high, max |
| claude-sonnet-4-6 | claude-sonnet-4-6 | low, medium, high, max |

`account` has the keys `["apiProvider","email","organization","subscriptionType"]`, which hold identity data only and no plan limits.

### 3. The `system/init` event does not list models or report effort

```sh
claude -p --model haiku --output-format stream-json --verbose --no-session-persistence "Reply with: ok"
```

`init` keys: `agents, analytics_disabled, apiKeySource, capabilities, claude_code_version, cwd, fast_mode_disabled_reason, fast_mode_state, mcp_servers, memory_paths, messaging_socket_path, model, output_style, per_turn_effort_active, permissionMode, plugins, product_feedback_disabled, session_id, skills, slash_commands, subtype, terminal_slash_commands, tools, type, uuid, view_mode`. None of them is an `effort` key.

```json
{"model":"claude-haiku-4-5-20251001",
 "capabilities":["interrupt_receipt_v1","interrupt_cancel_queued_v1","interrupt_send_now_v1",
   "msg_lifecycle_v1","sdk_mcp_tools_list_changed","sdk_mcp_manifests","mcp_read_resource_v1",
   "mcp_tool_ui_meta_v1","ui_surface_v1"],
 "per_turn_effort_active":false,"apiKeySource":"none","claude_code_version":"2.1.289"}
```

- `model` is the resolved ID of the model that runs.
- `capabilities` lists protocol feature flags, not models.
- `per_turn_effort_active` depends on the model, not on whether `--effort` was applied:

| run | `per_turn_effort_active` |
|---|---|
| `--model sonnet` (no `--effort`) | true |
| `--model sonnet --effort low` | true |
| `--model haiku --effort high` | false |
| `--model claude-sonnet-4-6 --effort high` | false |
| `--model claude-sonnet-4-6 --effort xhigh` | false |

The flag stays false for Sonnet 4.6 with a level it supports (`high`), so the field cannot confirm that effort was applied.

### 4. `--effort` with an unknown level warns and carries on

```sh
claude -p --model haiku --effort turbo --output-format stream-json --verbose --no-session-persistence "Reply with: ok"
```

```
exit=0
stderr: Warning: Unknown --effort value 'turbo' — ignoring it and using the default effort. Valid values: low, medium, high, xhigh, max.
```

The run then completes as normal. The only signal is that stderr line.

### 5. `--effort` with a level the model does not support is accepted silently

| command | exit | stderr | result |
|---|---|---|---|
| `--model haiku --effort high` | 0 | (empty) | `success`, `"ok"` |
| `--model claude-sonnet-4-6 --effort xhigh` | 0 | (empty) | `success`, `"ok"` |

The CLI gives no warning and no error. A `--debug-file` log of the Sonnet 4.6 `xhigh` run has no line that mentions effort, so whether the level was dropped, clamped or sent can't be observed from the CLI.

### 6. An unavailable model fails fast

```sh
claude -p --model claude-nonexistent-9 --output-format stream-json --verbose --no-session-persistence "Reply with: ok"
```

```
exit=1
stderr: [claude-code:unrecognized_model] {"model":"claude-nonexistent-9","query_source":"sdk"}
{"type":"system","subtype":"init","model":"claude-nonexistent-9"}
{"type":"assistant","error":"model_not_found","text":"There's an issue with the selected model (claude-nonexistent-9). It may not exist or you may not have access to it. Run --model to pick a different model."}
{"type":"result","subtype":"success","is_error":true,"api_error_status":404,"duration_ms":1340,"total_cost_usd":0, ...}
```

Note that `init` echoes the bad name back unchanged, so `init` alone does not show that the model is invalid. The run fails at the first API call, about 1.3 s in.

## Implications for #6 (model and effort pickers)

1. **Fill both pickers by query, not from a curated list.** At startup, and again on demand, spawn `claude -p --input-format stream-json --output-format stream-json --verbose --no-session-persistence`, write one `initialize` control request, close stdin, and read the `control_response`. It costs nothing in plan usage, because no model is called. Cache the result per CLI version (`claude --version`).
2. **The model picker** shows `displayName` (with `description` as a hint) and stores `value` for `--model`. The `resolvedModel` field shows what an alias such as `opus` currently points to. Store the alias if runs should track the latest model, or the resolved ID if they should be pinned.
3. **The effort picker** comes from the selected model's `supportedEffortLevels`. Hide or disable it when `supportsEffort` is missing (Haiku). Validate before spawning, because the CLI accepts invalid levels silently or with only a stderr warning.
4. **Revalidate saved selections.** On load, check each saved model against the current `models` list, since models can be added or removed between CLI versions. A stale selection that slips through still fails clearly: treat `assistant.error == "model_not_found"` or `result.api_error_status == 404` as "model unavailable" and point the user back to the picker.
5. **Run Inspector:** `init.model` is a reliable record of the model that ran. Nothing in the stream records the effort used, so the app has to record the effort it passed itself.
6. **The runner moves off thinking tokens.** `src-tauri/runners/src/command.rs` passes `--max-thinking-tokens` today. The effort spec should decide whether `--effort` replaces it, or how the two interact.
7. **Not established:** whether the list is filtered by plan. Only one account and plan was probed here, and no expensive listed model (Fable, for example) was run to confirm access. If the list turns out not to be filtered, the `model_not_found` handling in item 4 covers that gap.
