# Worker permission mode: `--permission-mode auto` under headless `--print`

Question: should agent-bus workers run with `--permission-mode auto` so that they can
`git push` a branch or `gh pr checkout` when the task asks for it, while the auto-mode
classifier blocks risky actions? This note answers six sub-questions with live runs of the
installed CLI and the official docs.

## Method

- CLI: `claude` **2.1.292**, subscription OAuth (`apiKeySource: "none"` in every `init` event).
- Ten runs, all in a throwaway repo under a scratch directory. `origin` and `backup` were
  **local bare repos** (file-path remotes). Nothing touched a network remote.
- Every run used the worker command line, plus a model and an optional extra flag:

  ```
  claude --print --output-format stream-json --verbose --model <m> \
    --settings <file> --setting-sources= --strict-mcp-config \
    --permission-prompts none --permission-mode auto --add-dir=<scratch>/adddir \
    [extra] "<prompt>" < /dev/null
  ```

  Without `< /dev/null`, the CLI waits 3 s and prints `Warning: no stdin data received in 3s,
  proceeding without it.` on stderr. The runner should close stdin.
- Sources:
  - [Permission modes](https://code.claude.com/docs/en/permission-modes), the "Eliminate
    permission prompts with auto mode" section. Cited below as **[PM]**.
  - [Configure auto mode](https://code.claude.com/docs/en/auto-mode-config). Cited below as **[AMC]**.
  - `claude auto-mode defaults` and `claude auto-mode config` output from the CLI itself.

| Run | Model | Settings | What it tested | Outcome |
|---|---|---|---|---|
| r1 | haiku | `{}` | Is the model supported? | **Silently ran in `default` mode** |
| r2 | sonnet | `{}` | Edits in cwd and add-dir, plus a branch push | All allowed |
| r3 | sonnet | deny `Bash(git push:*)` and `Edit(//…/adddir/**)` | Deny rules on top of auto | Both denied |
| r4 | sonnet | `autoMode.hard_deny` += "no git push" | A classifier block | Push blocked by classifier |
| r5 | sonnet | r4 + allow `Bash(git push origin feature-z)` | Allow rule vs. classifier | Push ran |
| r6 | sonnet | `{}` | `git remote add` + push, task only in `--append-system-prompt` | Allowed |
| r7 | sonnet | `autoMode.soft_deny` += "push only if the user asks" | Task only in `--append-system-prompt` | Allowed |
| r8 | sonnet | same as r7 | The push command only in a file the agent reads | **Blocked** |
| r9 | sonnet | same as r7, with `CLAUDE_CODE_AUTO_MODE_SERVER=0` | Same as r8, with a client-side classifier | Allowed (verdict differed from r8) |
| r10 | sonnet | r4 settings, with `CLAUDE_CODE_AUTO_MODE_SERVER=0` and `--debug-file` | Classifier latency and cost | Blocked; the debug log shows the classifier calls |

## 1. Does auto work under `--print`, subscription auth, `--setting-sources=`?

**Yes, with a supported model. With an unsupported model it silently falls back to
`default`.**

- On sonnet (`claude-sonnet-5-5`), the `init` event reports `"permissionMode": "auto"`. Stderr
  stays empty, apart from the stdin warning noted above. `--setting-sources=` has no effect on
  whether auto is available.
- On haiku (r1), the stream shows the fallback, and stderr shows no error or warning:

  ```json
  {"type":"system","subtype":"status","status":null,"permissionMode":"default",...}
  {"type":"system","subtype":"init",...,"permissionMode":"default","model":"claude-haiku-4-5-20251001"}
  ```

  In `default` mode with `--permission-prompts none`, every edit and every non-read-only
  command is auto-denied. **A worker configured with haiku plus auto would quietly lose edit
  rights.** [PM] gives the model requirement: "Claude Opus 4.6 or later, Sonnet 4.6 or later,
  or a Fable model ... Older models, including Sonnet 4.5, Opus 4.5, Haiku, and claude-3
  models, are not supported on any provider."
- Prerequisites from [PM]:
  - **Plan.** "Plan: All plans."
  - **Organization.** On Team and Enterprise plans, an administrator can turn auto mode off with
    `permissions.disableAutoMode: "disable"`.
  - **No opt-in flag.** `CLAUDE_CODE_ENABLE_AUTO_MODE` "has no effect from v2.1.207 onward".
- With `--settings`, the debug log confirms the gate:

  ```
  [auto-mode] verifyAutoModeGateAccess: enabledState=enabled disabledBySettings=false model=claude-sonnet-5-5 modelSupported=true ... canEnterAuto=true
  ```

- **Runner action:** after spawning, assert that `init.permissionMode == "auto"`. If it isn't,
  fail or warn loudly, or refuse auto for unsupported models up front.

## 2. What does the worker see when the classifier blocks?

There are three signals, all in the stream.

**(a) A `system` event with `subtype: "permission_denied"`.** In r4 the review ran server-side,
which is the default on this version:

```json
{"type":"system","subtype":"permission_denied","tool_name":"Bash","tool_use_id":"toolu_017G…",
 "decision_reason_type":"classifier",
 "decision_reason":"The server-side auto mode classifier judged this action dangerous (it gave no explanation)",
 "message":"Permission for this action was denied by the Claude Code auto mode classifier. Reason: …"}
```

With the client-side classifier (r10), the reason is `"decision_reason":"Blocked by classifier"`.
In both cases, the rule name (such as `[Data Exfiltration]`) **was not** included.

**(b) The `tool_result`.** It has `is_error: true` and carries the same `message` text. That text
tells the model to "Get as much of the rest of the task done as you can, then STOP and explain to
the user what you were trying to do", and not to pursue the same outcome another way. In every
blocked run, the worker kept going, finished the other steps, and reported the denial in its
final text.

**(c) `result.permission_denials`.** The blocked call appears there in the same shape as a
rule-based denial:

```json
"permission_denials":[{"tool_name":"Bash","tool_use_id":"toolu_017G…","tool_input":{"command":"git push origin feature-z"}}]
```

`permission_denials` **does not** say whether a deny rule or the classifier caused the denial.
The `system/permission_denied` event does, through `decision_reason_type`. r3 shows the
rule-based value:

```json
{"type":"system","subtype":"permission_denied","tool_name":"Bash","decision_reason_type":"subcommandResults",
 "message":"Permission to use Bash with command git push origin feature-y has been denied."}
```

In r3, a denied `Write` produced **no** `system/permission_denied` event. It produced only a
`tool_result`:

```
is_error=true  <tool_use_error>File is in a directory that is denied by your permission settings.</tool_use_error>
```

It was still listed in `permission_denials`. So `permission_denials` is the complete list, and
the system event adds the reason where one exists.

The run still ends with `subtype:"success"` and `is_error:false`. A block does not fail the
run. [PM] adds that under `-p` the repeated-block thresholds (3 in a row, 20 in total) have no
prompt to fall back to: "the action doesn't run and Claude keeps working. Claude Code doesn't
stop the run."

Other events seen in these runs:

- With the client-side classifier only, `{"type":"system","subtype":"permission_check_status","status":"checking"|"done"}`
  brackets each classifier check.
- `{"type":"system","subtype":"vcs_state_changed","kind":"commit"|"push","branch":…}` fires on
  every commit and push in every mode. It is useful for activity logs.

## 3. Do `--settings` allow/deny rules still apply on top of auto?

**Yes, and they take precedence over the classifier in both directions.**

- **Deny wins.** In r3, `Bash(git push:*)` in `permissions.deny` blocked `git push origin feature-y`
  with `decision_reason_type:"subcommandResults"`. The classifier was never consulted. [AMC]:
  `permissions.deny` "Blocks before the classifier is consulted. Neither the classifier nor user
  intent can override it."
- **Allow wins, even over a classifier hard_deny.** In r5, the r4 settings (a `hard_deny` against
  pushes) plus `permissions.allow: ["Bash(git push origin feature-z)"]` let the push run. The
  stream showed no `permission_denied` event, and `permission_denials` was `[]`. [PM], decision
  order step 1: "Actions matching your allow, ask, or deny rules resolve immediately".
- **Exceptions.** On entering auto mode, broad allow rules are dropped:
  - `Bash(*)`
  - wildcarded interpreters
  - package-manager run commands
  - `Agent` rules
  - `Monitor` rules

  Narrow rules like `Bash(npm test)` stay in effect. `autoMode.classifyAllShell: true` suspends
  every Bash allow rule [PM][AMC].
- **Bash rule limits.** Prefix rules match how the command begins. [AMC] notes that
  `git -C <dir> push` or `git -c k=v push` does not match `Bash(git push *)`. A deny rule on
  `git push` is not a complete barrier, and neither is an allow rule a complete grant.
- **Ask rules.** A content-scoped `permissions.ask` such as `Bash(git push *)` "always force[s] a
  permission prompt, even in auto mode" [AMC]. With `--permission-prompts none`, that prompt is
  auto-denied, so in practice an ask rule behaves like deny for a worker.

## 4. Edits inside `--add-dir` dirs, and does a deny `Edit(...)` still win?

**Yes on both counts.**

- In r4, the `Write` tool created `<scratch>/adddir/out3.txt` with no prompt and no classifier
  check. [PM], decision order step 2: "Read-only actions and file edits in your working
  directory are auto-approved". `additionalDirectories` count as working directories, as they
  do for acceptEdits.
- In r3, the deny rule `Edit(//<abs>/adddir/**)` won (the `//` prefix means an absolute path).
  The `tool_result` read `File is in a directory that is denied by your permission settings.`
- **Caveat.** An `Edit(...)` deny rule binds only the file tools. In r2, the model wrote into
  the add-dir with `echo hello > …/adddir/out.txt` in a Bash call. In auto mode that Bash call
  goes to the classifier, and the `Edit` deny rule does not apply to it.
- **Unlike acceptEdits:** writes to protected paths (`.claude/settings*.json`, `CLAUDE.md` and
  similar) route to the classifier rather than being auto-approved [PM].

## 5. Cost and latency of classifier calls

**On this subscription and version, classifier calls do not show up in `result.usage`,
`modelUsage`, `total_cost_usd` or `num_turns`. They do add wall-clock time.**

- **Server-side review is the default here.** For `-p`/SDK sessions on v2.1.281+, "Claude Code can
  ask the server to check the actions ... as part of the session's model requests, in place of
  sending its own classifier requests" [PM]. In r2 through r8, `modelUsage` lists only
  `claude-sonnet-5-5`, and `decision_reason` says "server-side auto mode classifier". [PM]: "there
  are no separate classifier calls to count".
- **The client-side classifier (`CLAUDE_CODE_AUTO_MODE_SERVER=0`, r10)** made two calls per
  checked action, with stages `xml_s1` then `xml_s2`, on `claude-sonnet-5[1m]`:

  ```
  [auto-mode] context comparison: ... classifierChars=137997 (sys=137802 tools=48 user=147) ...
  classifier_request_started  ... tool=Bash model=claude-sonnet-5[1m] stage=xml_s1
  classifier_request_finished ... stage=xml_s1 outcome=ok durationMs=2447
  classifier_request_started  ... stage=xml_s2
  classifier_request_finished ... stage=xml_s2 outcome=ok durationMs=2564
  ```

  That is about 5 s per checked shell command. It shows as a gap between `duration_ms` (11890)
  and `duration_api_ms` (6278). `modelUsage` still listed only the main model.
- **Billing.** [PM]: classifier calls count toward token usage "On Enterprise plans and on
  accounts that use the Claude API ...". On a subscription plan they don't count, and in any case
  they are invisible in the stream.
- **Which actions reach the classifier.** Reads and edits in working directories skip it. Only
  shell, network and other non-trivial actions are checked, so the overhead scales with the
  number of Bash calls.
- `num_turns` is unaffected by the classifier itself. A block does add a turn, because the model
  reacts to the denial.

## 6. Can the classifier be given context or configuration?

**Configuration: yes, via `--settings`, and it works with `--setting-sources=`.**

- `autoMode` is read from `~/.claude/settings.json`, from managed settings, and from "`--settings`
  flag or Agent SDK — Inline JSON — Per-invocation overrides for automation". It is **not** read
  from project `.claude/settings*.json` [AMC].
- The keys are `environment`, `allow`, `soft_deny` and `hard_deny` (prose rules) and
  `classifyAllShell`. Include `"$defaults"` in a list to keep the built-in entries. Setting a list
  without it **replaces** the defaults for that section [AMC].
- To verify the effective config for a worker's settings file, run
  `claude --settings <file> --setting-sources= auto-mode config`. Global flags must come before
  the subcommand: `claude auto-mode config --settings …` fails with `unknown option '--settings'`.
  With the r4 file, this printed `hard_deny` = `["Data Exfiltration: …", "No Git Push: …"]`.
- Runs r4 and r8 show that rules passed through `--settings` reach the server-side classifier.

**Defaults already allow what the workers need.** The built-in allow rule
`Git Push Destination` reads: "Pushing to any branch of the session's repo is ordinary — the
default branch included". [AMC] adds "and pull request creation by default". In r2, a plain
`git push origin feature-x` ran with no special context. Pushes are judged on content
(secrets, code that leaks when run). The other limits are:

- Force-pushes to shared or default branches are blocked (soft rule `Git Destructive`).
- `git remote add` / `set-url` is not routine (`Remote Repoint`).
- Branches named like deploy targets (`production`, `release`, `gh-pages`) get no blessing.
- **Public repos:** under `Out-of-Place Publication`, the classifier blocks content that doesn't
  belong in a public repo, including confidential material from outside the repo. Repository
  visibility is assumed private "unless the remote host and repo name indicate otherwise". The
  classifier can't see `gh repo view` output (tool results are stripped), so visibility has to be
  stated in a user message or in `autoMode.environment` [AMC].

**Task context: the user message is the documented channel.**

- [AMC]: "if the user's message directly and specifically describes the exact action Claude is
  about to take, the classifier allows it even when a `soft_deny` rule matches". `hard_deny` can
  never be cleared by intent. In r4, the push stayed blocked even though the user message asked
  for it.
- In the client-side classifier, the classifier sees "user messages, tool calls other than
  read-only lookups ..., and your CLAUDE.md content. Tool results are stripped" [PM]. The r10
  debug line `user=147` is consistent with that: only the short user prompt was counted.
- **Observed with the default server-side review:**
  - **r7:** a custom soft rule said "Block [git push] unless the user's own message explicitly
    asks". The push instruction was **only in `--append-system-prompt`**, and the user message was
    just "Do the task described in your system instructions." The push was **allowed**.
  - **r8:** the same rule with the push command only in a file the worker read. The push was
    **blocked**.
  - **r6:** with default rules, `git remote add backup …` followed by `git push backup main`,
    both requested only through the appended system prompt, were **allowed**.

  So in these runs, server-side review credited the appended system prompt as the task's
  intent. **The docs don't promise this.** Putting the push request in the user message is the
  documented and safer channel.
- **The client-side classifier gave a different verdict** for the r8 scenario. With
  `CLAUDE_CODE_AUTO_MODE_SERVER=0`, the same file-sourced push was allowed in r9. Verdicts on
  borderline prose rules are not stable across classifier paths, and may not be stable across
  runs either. Use `permissions.deny` or `permissions.allow` for anything that must be
  deterministic.
- **Other context levers:**
  - CLAUDE.md content, which steers both Claude and the classifier [AMC]. Whether CLAUDE.md
    reaches the classifier when `--setting-sources=` is set was **not tested**.
  - A PostToolUse hook's `classifierContext` field [PM].
  - `autoMode.environment` entries, such as a "Repository visibility" or "Source control" slot,
    passed through `--settings`.

## Recommendation for agent-bus workers

1. Auto mode is viable for `--print` workers on a subscription, on Sonnet 4.6+ or Opus 4.6+
   only. Validate `init.permissionMode` after spawn, because haiku silently degrades to `default`.
2. Keep hard boundaries in `permissions.deny`, for example force-push, `gh pr merge` and
   `gh repo create`. They are deterministic and run before the classifier. Remember the
   prefix-matching limits.
3. Allow-by-task:
   - **Option one:** when a task should push, say so in the **user message** with the exact
     branch. That is the documented intent channel.
   - **Option two:** add a narrow `permissions.allow` rule for the task's branch to the per-task
     settings file. That is deterministic.
   - For tasks that must not push, either add a deny rule or rely on the user message saying
     "don't push". [PM]: stated boundaries act as a block signal.
4. Because agent-bus-app is public, put a `"Repository visibility: …"` entry in
   `autoMode.environment` with `"$defaults"`, so the classifier scopes what may be pushed.
5. Detect classifier blocks from `system/permission_denied` events with
   `decision_reason_type:"classifier"`. Use `result.permission_denials` for the full list. Don't
   expect a rule name.
6. Budget roughly 0 to 5 s of extra latency per checked shell command. There is no visible
   token cost on a subscription.
7. Close stdin (`< /dev/null`) when spawning.
