# crewAI

crewAI is a terminal-first team coding assistant built on top of Herdr.
It combines local team coordination, signed task and broadcast records,
permission-aware repository requests, and bounded agent execution.

## Project layout

- `idea.md` — product idea and goals
- `spec.md` — architecture and security requirements
- `task.md` — implementation checklist and verification notes
- `herdr/` — application source
- `docs/` — operations, networking, and MVP walkthrough documentation
- `tools/build-herdr.ps1` — E:-drive build and test wrapper for Windows

## Build on Windows

Use the repository wrapper so Cargo, Rustup, temporary files, and build output
remain on E: and incremental compilation stays disabled:

```powershell
.\tools\build-herdr.ps1 -Mode build
.\tools\build-herdr.ps1 -Mode check
.\tools\build-herdr.ps1 -Mode fmt-check
```

The current implementation is a runnable developer MVP. The complete
two-device team onboarding flow, automatic reconnect workers, and an enforced
Windows read-only execution sandbox remain documented product limitations.
