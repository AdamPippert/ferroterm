Ferroterm — Product Requirements Document (PRD)

1  Situation Analysis
	•	Problem. Power users want an ultra‑fast, keyboard‑first terminal that embeds an AI agent natively, without the latency, web‑UI overhead, or cloud lock‑in of current “AI‑inside” shells. Warp is proprietary and macOS‑only; open‑source options lack integrated agent hosting and sub‑150 ms cold‑start times.
	•	Opportunity. Ship a cross‑platform (macOS + Linux) GPU‑accelerated Rust terminal whose command prefix p turns the rest of the line into LLM context, enabling one‑keystroke AI assistance and local model override via --model.
	•	Success Criteria.
	1.	Time‑to‑first‑frame ≤ 50 ms on M‑series and modern x86 laptops.
	2.	Agent response latency (local 7‑B model, 4‑bit quantized) ≤ 300 ms for 32‑token prompt.
	3.	Conformance to POSIX PTY semantics; all common shells and TUI apps behave identically inside ferroterm.
	4.	100 % of config in plain‑text (ferroterm.toml)—no GUI needed for full power.

⸻

2  Solution Architecture & Requirements

Layer	Component	Functional Requirements	Non‑Functional / Perf
Core Runtime	Rust TTY Engine (async‑pty)	Spawn & manage PTYs; relay I/O; handle signals; support raw, cooked, alt‑screen modes.	Zero‑copy buffers; < 1 ms round‑trip echo under load.
	Renderer (wgpu)	GPU text grid, ligatures, emoji fallback, true‑color; triple‑buffered.	144 FPS target; CPU < 5 % at idle with 4K window.
	Input/Keymap	Configurable keybindings (keymap.toml); p captured only at line start.	Must not break native readline/zsh key combos.
Agent Subsystem	Command Prefix Parser	Detect p / p --model <id>; collect rest of line as prompt; pass stdin snapshot & scrollback (configurable) as context.	O(1) prefix dispatch; no perceptible delay in regular shell commands.
	Model Host	Load local GGUF / MLC / vLLM models; fallback to remote endpoints (OpenAI, Gemini) via per‑model adapter.	Warm pool of N workers; cold‑start ≤ 150 ms; memory isolation.
	Streaming UI	Inline assistant output with ANSI dim color; supports markdown → plaintext render; abort on Ctrl‑C.	Sub‑frame latency between token receive and paint.
Config & State	Plain‑Text Config (~/.config/ferroterm/ferroterm.toml)	Sections: [ui], [keymap], [agent], [models], [telemetry].	Live‑reload; schema‑versioned; comments preserved.
	Profile Cache	Per‑model prompt templates, temperature, top‑p, system messages.	LRU; disk‑backed; encrypted if crypto = true.
Extensibility	Plugin SDK	Rust cdylib/Wasm plugins can register new prefix commands, custom panes.	Sandbox via WASI; plugin crash isolation.
Distribution	Installer	Homebrew (brew install ferroterm), Cargo (cargo install), .deb/.rpm.	Binary < 20 MB compressed; no runtime deps beyond libc, GPU drivers.
Telemetry (Opt‑In)	Minimal anonymized stats; error backtraces.	GDPR compliant; CLI flag --no‑telemetry.	
Security	Signed releases; supply‑chain SBOM; seccomp sandbox around model host.	Reproducible builds CI (GitHub Actions + Nix).	


⸻

3  Execution Framework

3.1 Milestones

Date	Deliverable
 T + 2 w	MVP CLI prototype — PTY pass‑through, p prefix echoes back prompt.
 T + 6 w	Alpha — GPU renderer, config file, local mistral‑7b‑instruct running via llamacpp.
 T + 10 w	Beta — Linux + macOS parity, plugin SDK v0, performance benchmarks published.
 T + 14 w	Release 1.0 — signed installers, docs, homebrew‑core acceptance.

3.2 Team & Ownership

Area	Owner	KPIs
Core Runtime & Renderer	Lead Rust engineer	FPS, CPU %
Agent Host	AI infra eng.	p99 latency
Config & SDK	Dev‑tools eng.	Plugin load time
Release Engineering	CI/CD eng.	Repro builds pass rate

3.3 Risks & Mitigations

Risk	Impact	Mitigation
GPU driver quirks on older Intel Macs	Crash / artifacts	Fallback software renderer; CI farm across GPU vendors
Model memory footprint (> 8 GB)	OOM on laptops	Default to 4‑bit quant + context window truncation
Prefix collision with user scripts	Usability	Configurable primary prefix; conflict detector on startup

3.4 Open Questions
	1.	Which tokenizers to bundle out‑of‑the‑box?
	2.	Ship default OSS model or prompt user for download on first run?
	3.	Licensing: Apache‑2.0 vs MIT + Commons Clause for model adapters?

⸻

Acceptance Test Checklist
	•	Typing p how to search git log? returns an AI answer within 500 ms.
	•	p --model gpt4 how to rebase? overrides default model for that invocation only.
	•	Regular command python runs unmodified.
	•	Config change ui.font_size = 14 hot‑reloads without restart.
	•	Renderer maintains ≥ 120 FPS scrolling yes | head -n 100000.

⸻

End of PRD
