# Security Policy

Desktop Pilot controls a real desktop, so security reports matter a lot to us.

## Reporting a vulnerability

Please **do not open a public issue**. Report privately via GitHub's "Report a vulnerability" (Security Advisories) on this repository. *(A dedicated security e-mail address will be added before the first public release.)*

Include: affected version and OS, steps to reproduce, impact, and any proof of concept. We aim to acknowledge reports within 3 working days.

## Scope

Of particular interest:
- Ways to make the agent perform risky actions without the confirmation dialog (risk-classifier bypasses).
- Prompt injection from on-screen content that leads to actions outside the user's request.
- FORCE STOP failures (input continuing, keys left held down).
- API key or other secret leakage (logs, files, crash reports, screenshots).
- Update or installer tampering.

The threat model and safety controls are documented in [docs/SECURITY.md](docs/SECURITY.md).

## Supported versions

Pre-release (0.x): only the latest commit on `main` is supported.
