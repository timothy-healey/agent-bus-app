# What `--bare`, `--strict-mcp-config` and `--disallowed-tools` change for a worker

Research for issue #9 (map #3). Feeds "Spec: real scope enforcement" (#10).

Every claim below comes from real `claude --print --output-format stream-json --verbose` runs of the installed CLI (`2.1.289`) under subscription (OAuth) auth, plus `claude --help` for that version. No `ANTHROPIC_API_KEY` or `apiKeyHelper` was set.

## Answer

**The "global permissions leak" was not a leak.** The user settings hold no `permissions` block at all. The earlier probe ran `echo`, a read-only command that Claude Code approves without an allow rule. A command that is not read-only (`python3 -c 'print(6*7)'`) is denied under the app's current args, and the denial lands in `result.permission_denials`. So the app's `--settings` allow list is already enforced for anything that needs approval. It does not stop read-only Bash commands.

**`--bare` is not usable on subscription auth.** It never reads OAuth or the keychain, so every run ends with `Not logged in`. Setting `CLAUDE_CODE_OAUTH_TOKEN` does not change that. `--setting-sources=` (empty) plus `--plugin-dir` gets what `--bare` was wanted for, and auth keeps working: no user hooks, no user-installed plugins, and only the plugins named explicitly (e.g. `ddd-council:ddd-council`).

**`--disallowed-tools Bash` removes Bash from the tool list.** So does a `"deny": ["Bash"]` rule in `--settings`. This also applies to subagents. Nothing is recorded in `permission_denials`, because the call never reaches a permission check. `--strict-mcp-config` changed nothing here, because no MCP servers show up in a `--print` worker on this machine to begin with.

Base args for every run: `--settings <tmp>/worker.settings.json --permission-mode acceptEdits --model haiku`, with the cwd set to a fresh `mktemp -d`. This matches `build_args` in `src-tauri/runners/src/command.rs`. Settings file: `{"permissions":{"allow":["Read"],"deny":["Bash(git push:*)","Bash(git fetch:*)"]}}`, which is the shape `build_settings` in `scope.rs` produces.

| # | Extra flags | Auth works | User hooks fire | Skills in init / `ddd-council:ddd-council` | Command asked for | Blocked? | `permission_denials` |
|---|---|---|---|---|---|---|---|
| A1 | none (app today) | yes | yes (SessionStart) | 83 / yes | `echo probe-ok` (read-only) | **no**: ran, printed `probe-ok` | `[]` |
| A2 | none (app today) | yes | yes | 83 / yes | `python3 -c 'print(6*7)'` | yes: "This command requires approval" | 1 entry (Bash, the command) |
| B1 | `--disallowed-tools=Bash` | yes | yes | 83 / yes | `echo probe-ok` | yes: Bash absent from `tools`; the model tried `Agent`, and the subagent had no Bash either | `[]` |
| F1 | settings `"deny":["Bash"]` (no flag) | yes | yes | 83 / yes | `echo probe-ok` | yes: "No such tool available: Bash. Bash is disabled for this session, in subagents as well as here." | `[]` |
| G1 | `--tools=Read,Edit` | yes | yes | 83 / yes, but **no `Skill` tool** | `echo probe-ok` | yes: only 2 tools exist | `[]` |
| D1 | `--strict-mcp-config` | yes | yes | 83 / yes | `python3 …` | yes (same as A2) | 1 entry |
| E1 | `--permission-prompts none` | yes | yes | 83 / yes | `python3 …` | yes, with a clearer message ("no approval surface") | 1 entry |
| C1 | `--bare` | **no**: `Not logged in · Please run /login` | no | 18 built-ins / **no** | n/a | n/a | `[]` |
| C2 | `--bare --plugin-dir <ddd-council>` | **no** | no | 60 / yes (other user plugins came back too) | n/a | n/a | `[]` |
| J1 | `--bare` + `CLAUDE_CODE_OAUTH_TOKEN=<dummy>` | **no**: same `Not logged in` | no | 18 / no | n/a | n/a | `[]` |
| I1 | `--safe-mode --plugin-dir <ddd-council>` | yes | no | 19 / **no** (plugin-dir ignored) | `echo probe-ok` | no (read-only) | `[]` |
| H1 | `--setting-sources=project` | yes | **no** | 19 / no (user plugins not enabled) | `python3 …` | yes | 1 entry |
| H2 | `--setting-sources=project --plugin-dir <ddd-council>` | yes | **no** | 20 / **yes** | `python3 …` | yes | 1 entry |
| K1 | `--setting-sources= --plugin-dir <ddd-council>`, settings allow `Bash(python3 -c:*)` | yes | **no** | 20 / **yes** | `python3 …` | no: allowed rule ran it, printed `42` | `[]` |

Here "skills" means the length of `system/init.skills`. `slash_commands` gave the same `ddd-council` result as `skills` in every run.

## Evidence

### Harness

Each probe made a fresh temp dir and wrote the settings file into it. It then ran this from inside the dir, saved the stream, and deleted the dir:

```sh
dir=$(mktemp -d); cd "$dir"
claude --print --output-format stream-json --verbose \
  --settings "$dir/worker.settings.json" --permission-mode acceptEdits --model haiku \
  <extra flags> \
  "Use the Bash tool exactly once to run: <cmd> . If the tool call is denied or unavailable, do not retry and do not use any other tool; reply with the single word DENIED. Otherwise reply with only the command output."
```

The summaries were pulled with `jq`, using `system/init` (`apiKeySource`, `tools`, `plugins`, `skills`, `slash_commands`, `mcp_servers`), `system` events whose subtype starts with `hook`, the `tool_use` and `tool_result` blocks, and `result`. Hook output contents, session IDs and tool-use IDs are left out below.

What the user config holds (checked before probing, read-only): `~/.claude/settings.json` has no `permissions` key and no `hooks` key. `~/.claude/settings.local.json` has `hooks`. Enabled plugins include one with a SessionStart hook. There is no managed settings file.

### A1 / A2: the app's current args

```
A1 hook events: hook_started:SessionStart x1, hook_response:SessionStart x1
A1 init: {"apiKeySource":"none","permissionMode":"acceptEdits","tools_n":30,"has_Bash":true,"mcp":[],"skills_n":83,"ddd_skill":["ddd-council:ddd-council"]}
A1 tool: Bash {"command":"echo probe-ok"} -> is_error:false "probe-ok"
A1 result: {"subtype":"success","is_error":false,"result":"probe-ok","permission_denials":[]}

A2 tool: Bash {"command":"python3 -c 'print(6*7)'"} -> is_error:true "This command requires approval"
A2 result: {"subtype":"success","is_error":false,"result":"DENIED",
  "permission_denials":[{"tool_name":"Bash","tool_use_id":"…","tool_input":{"command":"python3 -c 'print(6*7)'"}}]}
```

`apiKeySource: "none"` is how subscription OAuth auth shows up in a run that works.

### B1: `--disallowed-tools`

The first attempt passed `--disallowed-tools Bash "<prompt>"` and failed with `Error: Input must be provided either through stdin or as a prompt argument when using --print`. The flag is variadic (`<tools...>`), so it swallowed the prompt. `--disallowed-tools=Bash` fixed it:

```
B1 init: {"tools_n":31,"has_Bash":false, ...}
B1 tool calls: Agent {"prompt":"Run this exact command and report the output: echo probe-ok"}
               (subagent) ToolSearch "bash shell run command"; ToolSearch "select:Bash" -> "No matching deferred tools found"
B1 subagent report: "I don't have access to a Bash tool in my current toolset…"
B1 result: {"result":"DENIED","permission_denials":[]}
```

`Agent` is not in the allow list, but the worker could still spawn a subagent. The subagent inherited the disallow.

### F1: settings `deny: ["Bash"]`

```
F1 init: {"has_Bash":false, ...}
F1 tool: Bash {"command":"echo probe-ok"} -> "<tool_use_error>Error: No such tool available: Bash. Bash is disabled for this session, in subagents as well as here.</tool_use_error>"
F1 result: {"result":"DENIED","permission_denials":[]}
```

A bare tool name in `deny` acts like `--disallowed-tools`: the tool is removed, not denied per call.

### G1: `--tools`

```
G1 init: {"tools_n":2,"has_Bash":false,"skills_n":83,"ddd_skill":["ddd-council:ddd-council"]}
G1 result: {"result":"DENIED","permission_denials":[]}
```

With `--tools=Read,Edit`, skills are still listed, but the `Skill` tool is gone, so the worker cannot invoke them. An allow-list built with `--tools` has to include `Skill` if skills are wanted.

### D1 / E1: `--strict-mcp-config`, `--permission-prompts none`

D1 matched A2 exactly (same init, same denial). `mcp_servers` was already `[]` in every `--print` run here, so the flag had nothing to strip on this machine. It matters only when user or project MCP config exists.

E1 matched A2, except for the tool_result text: `"Permission for this tool use was denied. It requires approval, and this session has no approval surface — nobody can answer a permission prompt here — so it was …"`. The denial is recorded the same way.

### C1 / C2 / J1: `--bare`

```
C1 init: {"apiKeySource":"none","tools_n":3 (Bash, Edit, Read),"skills_n":18,"ddd_skill":[]}
C1 hook events: none
C1 result: {"subtype":"success","is_error":true,"result":"Not logged in · Please run /login","permission_denials":[]}
C2 (+ --plugin-dir <ddd-council>) init: {"skills_n":60,"ddd_skill":["ddd-council:ddd-council"]}; result: same "Not logged in"
J1 (+ CLAUDE_CODE_OAUTH_TOKEN=<dummy>) result: same "Not logged in"
```

This matches `claude --help`: "Anthropic auth is strictly ANTHROPIC_API_KEY or apiKeyHelper via --settings (OAuth and keychain are never read)". The process exits 1, but `result.subtype` is still `"success"` with `is_error: true`.

Two side effects:

- `--bare` also cuts the tool set to Bash, Edit and Read.
- Under `--bare`, `--plugin-dir` brought back the skills of *all* user-enabled plugins (60 skills), not just the named one.

### I1: `--safe-mode`

Auth worked and no hooks fired. `--plugin-dir` was ignored (19 built-in skills, no `ddd-council`). Not useful for workers that need a plugin skill.

### H1 / H2 / K1: `--setting-sources`

```
H1 (--setting-sources=project) hook events: none; init: {"skills_n":19,"ddd_skill":[]}; python3 denied, 1 permission_denials entry
H2 (+ --plugin-dir <ddd-council>) hook events: none; init: {"skills_n":20,"ddd_skill":["ddd-council:ddd-council"]}; python3 denied, 1 entry
K1 (--setting-sources= (empty) + --plugin-dir; allow "Bash(python3 -c:*)")
   hook events: none; init: {"skills_n":20,"ddd_skill":["ddd-council:ddd-council"]}
   tool: Bash python3 -> is_error:false "42"; result: {"result":"42","permission_denials":[]}
```

Leaving out the `user` source drops the user's hooks (from `settings.local.json` and the plugin SessionStart hook) and `enabledPlugins`, so only built-in skills remain. `--plugin-dir` then adds exactly the named plugin. `--settings` still applies, and both its allow and deny rules work.

## Implications for "Spec: real scope enforcement" (#10)

1. **Today's allow list works, but only for commands that need approval.** Read-only Bash (`echo`, `ls`, `cat` and similar) runs whatever `allow` says. If a team's scope must forbid shell access completely, leave Bash out of the tool set (`--tools`, `--disallowed-tools=Bash`, or a bare `"Bash"` in `deny`). Allow-listing other tools is not enough.
2. **Two kinds of block, with different signals.**
   - A per-call denial (the tool exists but needs approval) appears in `result.permission_denials` with the tool name and input. The Run Inspector can show it.
   - A removed tool (`--tools`, `--disallowed-tools`, or a bare deny) never appears there. The model just gets "No such tool available".
   - So the spec should not treat an empty `permission_denials` as proof the worker stayed in scope.
3. **Do not use `--bare` for workers.** It cannot authenticate with the subscription, and the map says the runtime is subscription only. To get a clean worker, use `--setting-sources=` (or `project`, if the target repo's project settings should apply) plus one `--plugin-dir` per plugin the team's prompts need. This drops user hooks and user-wide plugins, keeps auth, and keeps `--settings` enforcement.
4. **Add `--permission-prompts none`.** It changes nothing about what gets blocked, but it makes the denial message honest ("no approval surface"), which helps a worker give up cleanly.
5. **`--strict-mcp-config` is cheap insurance.** No MCP servers appeared in `--print` here, but a user or project `.mcp.json` would bring them in, so the spec can pass it on every run.
6. **Variadic flags swallow the prompt.** `--disallowed-tools`, `--allowed-tools`, `--tools`, `--add-dir` and `--mcp-config` take `<values...>`. `--plugin-dir` takes a single `<path>` and is repeated instead. `build_args` pushes the user message last, so any variadic flag added there must use the `--flag=value` form, or sit before a non-variadic flag. Today `--add-dir` is followed by `--permission-mode`, which is why it works.
7. **`Agent` is not gated by the allow list.** A worker can spawn subagents with only `Read` allowed. Subagents inherit removed tools (B1, F1), so a Bash ban still holds. If the spec wants no subagents at all, it has to remove `Agent` explicitly.
8. **`--tools` also removes `Skill`.** If teams pin a tool set with `--tools`, the spec must add `Skill` for skills like `ddd-council:ddd-council` to be invokable.
9. **Not tested:** `--bare` with a real `ANTHROPIC_API_KEY` or `apiKeyHelper`, since that is outside the subscription-only runtime. Also not tested: how project `.claude/settings.json` hooks in a target repo behave under `--setting-sources=project`, because the temp dir had none.
