# PRD P0-006 – Privacy & Telemetry Controls

## Mandate
Ferroterm MUST work 100 % offline by default.

## Functional Requirements
* Telemetry disabled on first-run.
* Opt-in via `pterm consent --telemetry basic|full`.
* Granular flags (`performance`, `crash`, `usage`).
* Events queued in local SQLite until user pushes.

## Success Metrics
* Static scan: zero outbound sockets when telemetry off.
* ≥ 25 % alpha users opt-in (trust indicator).

## Event Schema
```sql
CREATE TABLE events(
  ts INTEGER,
  category TEXT,
  payload BLOB
);

Risks
	•	Reduced observability – mitigate by surfacing manual crash upload cmd.
