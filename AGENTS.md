# Guidance

This project includes a providers.json file. It is a direct export from models.dev/api.json and lists all the providers
and models that can be accessed from this project. DO NOT READ THIS FILE DIRECTLY. If you need something from this file
then use a combination of tools like sed, grep, and jq but loading the entire file is never necessary.

## Agent skills

### Issue tracker

Issues live as GitHub issues, managed via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
