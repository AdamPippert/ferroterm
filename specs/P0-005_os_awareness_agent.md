# PRD P0-005 – OS-Awareness Agent

## Purpose
Feed live system facts (packages, logs, services) to AI so generated commands are accurate and environment-specific (Lightspeed parity).

## Functional Features
1. On prompt invocation, gather snapshot JSON:  
   * **Linux** – `sysinfo`, `rpm/apt`, `dmesg --level=err`  
   * **macOS** – `brew list`, `spctl --status`  
   * **Windows** – `wmic product get name,version`
2. Inject snapshot into `<system>` tag of prompt (≤ 32 KB).
3. Trim oldest entries to stay within limit.

## KPI
* 90 % correctness on 50-task automated remediation harness.
* Snapshot generation ≤ 150 ms.

## Privacy
* Snapshots remain local unless user opts-in to telemetry upload.

## Risks
* Divergent command surfaces per OS – modular collector per family.
