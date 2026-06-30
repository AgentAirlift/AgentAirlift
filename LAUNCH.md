# Launch Checklist

Use this when the repo is ready for a concentrated 24-48 hour launch window.
Do not spread the first push across multiple weeks; the goal is star velocity.

## Before Launch

- README above the fold states the pain, proof, install path, and generated demo asset.
- `assets/agent-airlift-architecture.svg` is embedded in the README.
- `assets/agent-airlift-demo.gif` is embedded in the README and renders on GitHub.
- GitHub description and topics are current.
- `cargo test` and CI are green on `main`.
- FAQ covers AI APIs, uploads, native resume, dropped data, and provider-health fallback.
- Prepare a private first-100 list of users who already care about Codex, Claude Code, OpenCode, Kiro, Rust CLIs, or agent tooling.

## Asset Policy

Keep local generation helpers under `demos/`, but do not commit those scripts or
tapes. The public repository should contain the final launch assets under
`assets/`, not the local recording machinery.

## One-Line Pitch

Agent Airlift moves an AI coding session from a degraded provider to another
agent without losing context, decisions, tool history, or auditability.

Shorter variant:

Provider degraded? Airlift your AI coding session to another agent.

## Show HN Draft

Title:

```text
Show HN: Agent Airlift - move AI coding sessions between Codex, Claude Code, and OpenCode
```

First comment:

```text
I built Agent Airlift after getting stuck in long-running AI coding sessions where the provider or harness became the bottleneck.

The tool is deliberately local and boring: it imports a source session JSONL, normalizes it into a deterministic canonical schema, then emits HANDOFF.md, AGENTS.md, target-shaped exports, and audit reports. It does not call AI APIs during conversion and it does not upload artifacts.

The thing I wanted was not magic native resume. I wanted a reliable, inspectable handoff bundle that lets me move from Claude Code to Codex, Kiro, or OpenCode when a provider is degraded or when I want a second agent to continue the work.

The repo includes fixtures, generated launch assets, and CI gates that assert loss accounting and export sidecars match the canonical session.

Feedback I am especially looking for: which session formats should be supported next, and what evidence would make you trust a migrated agent handoff?
```

## Reddit Angles

r/rust:

```text
Show: Agent Airlift, a Rust CLI for deterministic AI coding-session handoffs
```

r/commandline:

```text
I built a local CLI that airlifts AI coding sessions between agent harnesses
```

r/selfhosted:

```text
Local-only AI session migration when a provider or agent harness degrades
```

Always disclose that it is your project. Do not ask for upvotes.

## Social Thread Skeleton

```text
Provider degraded in the middle of an AI coding session?

I built Agent Airlift: a local Rust CLI that moves a session from Claude Code or Codex into another agent without losing the working context.

[assets/agent-airlift-demo.gif]

What it preserves:
- current objective
- decisions and rationale
- tool calls/results
- repo context
- provider-health signal
- audit trail and dropped-field accounting

No AI APIs during conversion. No uploads. Explicit transfers only.

Repo: <link>
```

## Launch-Day Sequence

1. Post Show HN on Tuesday, Wednesday, or Thursday morning US time.
2. Immediately add the first comment with the origin story and technical constraints.
3. Post one Reddit thread in the best-fit subreddit first, not everywhere at once.
4. Post the social thread with the GIF; put the repo link in the thread/reply if the platform penalizes direct links.
5. Message the first-100 list asking for feedback, not votes.
6. Reply to every substantive comment for the next 6 hours.
7. If a critique is valid, open an issue immediately and link it in the response.

## Do Not Launch If

- The first screen of the README still needs explanation from you.
- The README GIF or architecture diagram is missing or stale.
- The install path is ambiguous.
- GitHub Actions is red.
- You cannot spend several hours responding.
