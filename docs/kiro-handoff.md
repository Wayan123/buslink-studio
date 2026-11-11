# Kiro ↔ omp handoff

Kiro builds BusLink Studio: specs, custom agents, hooks and steering in `.kiro/`, plus the code changes. omp (or any other terminal agent) only orchestrates. It starts Kiro sessions through the bridge, then reviews and verifies what Kiro produced. That split keeps the competition requirement honest: the work submitted is work Kiro did, and every session leaves evidence.

## How to run a Kiro task

```sh
# from the repository root (WSL or Linux shell)
python3 tools/kiro_bridge.py --agent ghostwire-builder --label "phantom-writer task 2" \
  "Execute task 2 of .kiro/specs/ot-security-lab/tasks.md"
```

- The bridge runs `kiro-cli chat --v3 --no-interactive --output-format stream-json`. It streams Kiro's answer and appends an entry to `docs/kiro-log/LEDGER.md` (agent, session ID, tools, changed files, credits). The raw transcript goes to `docs/kiro-log/runs/`.
- To continue a session, pass `--resume-id <session>` with the session ID from the ledger.
- Interactive work in the Kiro IDE or `kiro-cli` is equally valid. Add a short ledger entry by hand afterwards.

## Rules for the orchestrator

1. Do not edit product code or `.kiro/` directly. Ask Kiro to make the change. Direct edits are limited to `tools/`, `docs/kiro-log/` and release packaging, and each one must be listed in `docs/existing-code-disclosure.md`.
2. Review Kiro's diff and run the checks (`tools/dotnet run --project csharp/BusLink Studio.Tests/BusLink Studio.Tests.csproj`). If something fails, send the failure back to Kiro rather than patching it yourself.
3. Do not invent Kiro usage. `docs/kiro-usage.md` cites only ledger entries.
4. Commit after each verified Kiro task, with a message like `feat(kiro): <task> [session <id>]`.

## Environment

- Kiro CLI 3.2.0 or later, logged in (`kiro-cli whoami`).
- The .NET 8 SDK. The repository lives on `/mnt/d`, so `tools/dotnet` forwards to Windows `dotnet.exe`.
