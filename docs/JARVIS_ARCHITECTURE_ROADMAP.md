# Jarvis architecture review and learning roadmap

Date: 2026-09-27  
Revision: 3 — Tapo C100, household recognition, calendar, private recording/storage, context/token management, minimal boundary testing  
Reviewed code checkpoint: cc84ab5  
Contract: Astra reviews and guides; Darren implements. This document does not authorize implementation.

This is the consolidated plan, superseding the earlier chat roadmap. It preserves the original Rust, local/cloud LLM, tools, audio, C++, cameras, ML, devices, background-work, and controlled-improvement goals.

The complete original handoff and all current source files were inspected in the preceding review. This revision includes the user's Tapo C100 (already connected to their app), family recognition, calendar tools, possible Jarvis-managed private storage, and explicit context/token observability and optimization phases. Connection to the app is user-reported; a Jarvis RTSP integration is not implemented in the reviewed repository. No application source, dependencies, or camera/calendar settings were changed.

## 1. Assessment and verified baseline

**Keep the current foundation. Add narrow contracts and bounded execution, not a general agent framework.**

Current ownership and behavior:

~~~text
main → App → Assistant → LocalLlm → Rig → remote llama.cpp
       constructs         owns reusable completion and health clients

App.run: health check → fixed prompt → print response → exit
~~~

| Existing file | Actual responsibility | Review |
|---|---|---|
| [main.rs](/Users/darrenng/Desktop/Desktop/personalProjects/jarvis/src/main.rs) | Load configuration and launch App | Appropriately thin |
| [app.rs](/Users/darrenng/Desktop/Desktop/personalProjects/jarvis/src/app.rs) | Construct dependencies and run demonstration | Not presently a god object |
| [configuration](/Users/darrenng/Desktop/Desktop/personalProjects/jarvis/src/config/mod.rs) | Read three environment variables | Centralized; semantic validation is limited |
| [Assistant](/Users/darrenng/Desktop/Desktop/personalProjects/jarvis/src/services/assistant.rs) | Forward completion and health requests | Shallow today; bounded conversation gives it a useful responsibility |
| [LocalLlm](/Users/darrenng/Desktop/Desktop/personalProjects/jarvis/src/clients/local_llm.rs) | Provider construction, health, text extraction | Appropriate integration location; text-only return is insufficient for tools |
| [cloud placeholder](/Users/darrenng/Desktop/Desktop/personalProjects/jarvis/src/clients/cloud_llm.rs) | Empty, not declared in client module | Not a real second provider |

There are currently no tools, spawned application tasks, shared mutable application state, persistence, cameras, calendar integrations, facial recognition, or audio. There is no demonstrated race or god object in the present code.

The handoff's reported Pi success is consistent with the code. It was not independently rerun during this review. The deployed model, server flags, Pi configuration, and performance remain deployment-reported facts.

### Preserve

- Thin main, explicit construction, and concrete ownership.
- Reusable clients, returned results, and propagation of recoverable errors.
- Working Rig 0.42.0 / llama.cpp integration and current model baseline.
- Rust authority over validation, permissions, execution, and lifecycle.
- Two nodes: application/device host and AI/perception host.
- Separate training, inference, agent reasoning, and controlled promotion.
- Small runnable learning slices with explanations of ownership, borrowing, Result, ?, enums, traits when justified, and async behavior.

### Findings

1. **NOW: model output loses tool calls.** LocalLlm keeps only text and silently discards other content. A tool-only response becomes a no-text error; mixed output loses the call. Preserve a small structured model turn, including calls and correlation information.
2. **NOW: completion has no configured deadline.** The three-second timeout belongs only to the separate health client. Cached Rig 0.42.0 uses a default reqwest client when none is supplied; that client has no total request deadline configured.
3. **NOW, as tools arrive: authority and termination need code-level rules.** A schema or system prompt does not authorize execution.
4. **SOON: health semantics need tightening.** error_for_status rejects 4xx/5xx, not every unexpected response. Define expected success, redirect policy, and readiness meaning. Health does not prove tool support.
5. **SOON: configuration needs semantic checks.** Nonempty model IDs, valid endpoints/paths, and sensible limits should fail clearly at startup. Distinguish an absent optional dotenv file from a malformed one.
6. **SOON: introduce focused errors when decisions require them.** Box<dyn Error> is adequate for the demonstration; it is weak for distinguishing rejection, timeout, and failed execution. Cross-task errors may also require Send + Sync.
7. **SOON: record deployment facts.** Capture server revision, model checksum, flags, toolchain, and host setup without secrets. A model name alone is not reproducibility.
8. **LATER: perception identity and account authority must remain separate.** Recognizing a family member must not automatically expose their calendar or grant tools.

## 2. Critical architecture decisions

### Assistant and provider ownership

Assistant should directly own LocalLlm now. Give Assistant one job: conduct a bounded conversational run. Do not add AgentService and AgentRuntime as forwarding layers around it.

A narrow model trait is justified when a scripted test model or cloud implementation needs substitution. Two providers do not automatically require a trait: two instances of one adapter or an enum may suffice. Avoid implementing a provider framework.

App constructs dependencies and moves them into their owners. It need not retain every dependency itself. When workers arrive, App owns their lifecycle through a small supervisor.

### Deterministic operations and agents

Explicit commands, calendar reads, status collection, sensor access, STT/TTS, CV inference, tracking, schedules, and drivers are deterministic operations or inference workers. They are not separate agents.

An LLM may interpret a request or propose an action. Rust decides what may happen. Background events should call application operations directly, without inventing a conversation.

### Health and readiness

Keep protocol-specific health knowledge in its client. Move periodic checking to a supervised worker when Jarvis becomes long-running. Do not require an available LLM for direct status, calendar access, UI operation, or safe device behavior.

Two HTTP clients are not automatically duplication: health and generation can legitimately need different budgets. Explicit transport policy matters more than forcing one client everywhere.

### Meaningful boundaries, not maximum abstraction

Keep one Rust application crate initially. Separate processes for existing remote inference and C++ perception are justified by deployment and workload isolation. Separate every conceptual module into a process only when failure isolation or scheduling actually requires it.

Libraries supply HTTP, serialization, runtime, storage, codecs, numerical kernels, and inference. Darren implements routing, run state, validation, policy, event semantics, features, evaluation decisions, and integration.

## 3. Target ownership and dependency direction

~~~text
App: construction + lifecycle
 ├─ Assistant: one bounded run
 │   ├─ selected model adapter
 │   └─ run-local messages, budget, observations
 ├─ application operations: status / calendar / devices
 ├─ supervised workers: health / ingestion / scheduled work
 ├─ storage owner
 └─ interface adapters: CLI, later touchscreen/API/audio

Model output → untrusted proposal → validator/policy → operation
Operation → structured observation → optional model narration
~~~

~~~mermaid
flowchart TD
    U[CLI / touchscreen / voice / API] --> R[Trusted caller context and deterministic routing]
    R --> O[Application operations]
    R --> A[Assistant: bounded run]
    A --> M[Local or permitted cloud model]
    M --> V[Validate proposal and permissions]
    V --> O
    O --> D[Status / calendar adapter / device driver]
    D --> A
    C[Tapo local stream] --> P[C++ ingestion and perception]
    P --> F[Optional local face matching]
    P --> E[Versioned observations]
    F --> E
    E --> S[Rust state and persistence]
    S --> B[Deterministic background rules]
    B --> O
    S --> A
    X[Offline Python experiments] --> Y[Evaluated artifact]
    Y --> Z[Explicit promotion and rollback]
    Z --> P
~~~

Rules:

1. Tool validation, permissions, devices, calendar semantics, and observations do not depend on Rig.
2. Provider adapters translate protocols; they do not decide user authority.
3. Calendar adapters never receive arbitrary model-selected credentials, account URLs, or account access.
4. Face matching produces uncertain observations; it does not create an authenticated session.
5. UI and transport handlers translate inputs and outputs rather than owning business policy.
6. Training consumes explicit dataset exports and produces versioned artifacts.
7. Process messages are versioned data contracts, not language-specific object graphs.
8. Keep original tool-call correlation and any required provider continuation data inside the model boundary.

### Proposed modules — create only at the relevant phase

Root: /Users/darrenng/Desktop/Desktop/personalProjects/jarvis/

~~~text
src/
  main.rs
  app.rs
  config/mod.rs
  clients/local_llm.rs
  clients/cloud_llm.rs           # later
  services/assistant.rs
  model.rs                      # small turn/proposal contract
  context.rs                    # bounded request assembly; no memory framework
  tools/mod.rs                  # allowlisted validation and dispatch
  tools/system_status.rs
  tools/calendar.rs             # narrow tool entry points
  calendar/mod.rs                # query/write semantics and provider adapter
  routing.rs
  runtime.rs
  storage.rs
  media.rs                      # recording catalogue/control adapter later
  observations.rs
  household.rs                  # only when enrollment/household IDs exist
  devices/
  audio/
  interfaces/
perception/                     # separate C++ project; face module later
experiments/                    # offline Python
deploy/                         # deployment and artifact records
                               # recording bytes live outside the repository
docs/
~~~

Do not start with a generic integration/plugin framework, repository abstraction, global types module, or household graph database.

### God-object risk map

| Module | Warning sign | Responsibility limit |
|---|---|---|
| App | Per-request decisions and protocol logic accumulate | Assemble, start, stop |
| Assistant | Owns calendars, camera trackers, audio hardware, SQL | One conversational run |
| Context | Becomes a universal memory/retrieval/agent subsystem | Select and budget input; preserve provenance and protocol validity |
| Media | Reimplements an NVR, cloud drive, codecs, and auth | Recording metadata and bounded control of an established recorder |
| Runtime | Knows meaning of every event | Supervision and resource admission |
| Tool dispatcher | Dynamic universal integration engine | Validate and dispatch known capabilities |
| Household | Mixes identity guesses, credentials, permissions, schedules | Explicit household identifiers/enrollment mapping |
| Calendar | Scheduling agent, provider SDK, account policy collapse together | Bounded calendar operations with adapter separation |
| Perception | Owns home automation decisions | Frames → uncertain observations |
| Storage | Arbitrary SQL leaks everywhere | Concrete application storage operations |

## 4. NOW / SOON / LATER / AVOID

| Timing | Recommendation |
|---|---|
| NOW | Explicit completion deadline, structured model turns, one read-only tool, strict validation, bounded execution |
| NOW during loop slice | Minimal run ID, stages, outcome, timing; preserve observation separately from generated prose |
| SOON | Semantic config validation, focused errors, direct routing, worker supervision, readiness separation |
| SOON after first bounded loop | Token/latency accounting, request-context budget, bounded session history, measured flow comparison |
| SOON at first persistence | SQLite migrations, retention, deduplication, recovery behavior |
| LATER after tools | Read-only calendar; separate approved calendar writes |
| LATER after camera verification | One Tapo stream, one zone, then opt-in household recognition |
| LATER after live stream verification | Optional bounded local recording, then authenticated private remote viewing |
| LATER | Cloud route, devices, voice, UI/API, offline pattern learning and shadow deployment |
| LATER last | Controlled offline improvement proposals and deliberate promotion |
| AVOID | Arbitrary shell tool, autonomous tool repair, agent swarm, identity-based automatic permissions, silent cloud fallback, unbounded recording, Kafka/Redis by default |

### Exact NOW changes

| Change | Before → after | Existing files / new modules | Learning |
|---|---|---|---|
| Deadline | Assistant → unbounded model wait → finite wait | Existing LocalLlm; no new module | Futures, timeout result, ? |
| Structured turn | Provider output → String → Jarvis turn preserving proposals | LocalLlm, Assistant, App presentation; new model module | Structs, enums, moves, exhaustive matching |
| Read-only status | No tool → tools → OS observation | Main module declaration, App harness; new tools/status modules | Typed arguments/results, units, borrowing |
| Bounded loop | One request → proposal/validate/execute/observe/finalize | Assistant, LocalLlm, model, tools | Run-local state and transitions |
| Diagnostics | Final text only → run/stage/outcome/timing | App setup, Assistant, LocalLlm; manifest if adding tracing | Context and redaction |

Preserve unrelated code and the working provider integration while implementing these slices.

## 5. First tool-loop contract

system_status means the 8 GB application host, not the entire system. Accept exactly an empty argument object. Return uptime and selected memory measurements with units, source node, and observation time. Do not expose processes, environment variables, arbitrary files, or shell commands.

Initial limits:

- At most two model requests.
- At most one tool execution.
- Finite completion and whole-run deadlines.
- Bounded argument/result sizes and generated output.
- No repair loop, retries, parallel execution, or recursion.

~~~text
trusted request context
 → model proposes
 → Rust checks proposal count, name, shape, arguments and authority
 → deterministic tool executes once
 → original call + matching result returned to model
 → tools disabled for final response
 → stop
~~~

Reject malformed/unknown proposals, extra arguments, multiple calls, and further calls during finalization. Mixed text plus tool calls is intermediate output, not evidence of execution. Timeout does not prove the server stopped computing.

Keep a structured observation alongside prose: providing true data to a model does not guarantee it will repeat it accurately. Render important measurements deterministically.

Verify the existing model/server/template supports function calling before considering another model. See [llama.cpp function calling](https://github.com/ggml-org/llama.cpp/blob/master/docs/function-calling.md).

## 5A. Context management, token visibility, and flow optimization

Context management is an explicit learning track, not a side effect of adding conversation history. Observe the simple flow first, then enforce budgets, then introduce multi-turn history, and only then evaluate summarization.

### Ownership and visibility

Assistant owns run state and decides when a model request is needed. A small context module assembles the actual request from permitted inputs. The model adapter supplies provider usage metadata and knows provider-specific token counting. Storage optionally persists diagnostic summaries and explicitly permitted session data.

Preserve usage before LocalLlm converts a response into application data. The inspected Rig 0.42.0 Usage type includes input/output, cache-related and reasoning token counters. Preserve whether values were actually reported: a provider's missing counter must not silently become a measured zero.

Start with a compact CLI run report, later a UI view:

| View | Fields |
|---|---|
| Each model request | Run/request ID, stage, model/provider, input/output tokens, count source (reported/estimated/unavailable), duration, finish reason |
| Input breakdown | Policy/instructions, latest user message, selected history, tool schemas, tool observations, optional summary; estimates clearly labelled |
| Entire run | Model/tool call counts, cumulative reported input/output tokens, total latency, queue/tool/model time, completion/rejection/timeout |
| Context decisions | Effective window, reserved output, safety allowance, selected/dropped exchanges, truncated observations, compaction decision |
| Optimization comparison | Task success and observation correctness alongside tokens, latency, unnecessary calls, and optional cloud cost |

Do not log raw sensitive content just to count it. Token counts and content lengths normally suffice. Expose reasoning token counts only if available; do not display or persist hidden reasoning text. Cache and reasoning counters may be subsets of other counters, so do not blindly add all fields into one total. Label partial run totals when a cancelled/failed request has no usage.

Distinguish **current request context size** from **cumulative run usage**. Resending history increases cumulative input usage even if the context window stays the same size. Token savings, cached-token savings, latency savings, and provider billing are different measurements.

### Budget before every request

Use the effective context limit of the deployed model/server configuration, not an advertised model maximum. Server slot configuration may reduce the usable window.

~~~text
selected input tokens + reserved output tokens + safety allowance
    <= effective context window
~~~

Also bound cumulative run token use and model/tool calls independently. Where counts are estimates, reserve conservatively; do not present an exact spend guarantee from an estimate. Include any summarization call in run limits and telemetry.

For local inference, investigate the deployed llama.cpp server's tokenizer/template facilities; exact counting requires the same model tokenizer, chat template, special tokens and tool serialization as generation. Tokenizing message strings alone is not an exact request count. If the installed API cannot reproduce that assembly, label estimates, keep headroom, and handle context-limit rejection clearly. Do not assume an OpenAI tokenizer accurately counts Gemma. [llama.cpp server APIs](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md).

Priority when constructing a request:

1. Trusted policy/instructions and the current user request.
2. Required active tool-call/result exchanges and fresh observations for the current task.
3. Bounded recent completed exchanges.
4. Optional older summary or specifically retrieved data.

If required content cannot fit, return a clear size/budget error or request a narrower task. Do not silently drop instructions or the newest request. Trim complete old exchanges, never leave orphaned tool results. Truncate oversized observations structurally with a truncation marker; never cut JSON into invalid fragments.

Initially advertise only the tool schemas relevant to the permitted route. Do not send all future tools, all camera events, or an entire calendar to every request. Selection narrows exposure but never replaces execution-time validation.

### Memory and compaction

Separate run context, bounded conversation history, durable user-approved preferences, and authoritative external state. A conversation summary is not a calendar database, a biometric gallery, an approval record, or a source of current occupancy.

Start with an in-memory session and maximum history size. Add persistence only when restart continuity is wanted, with session/account isolation and deletion. Retrieve bounded fresh calendar/occupancy data through operations rather than accumulating all observations in history.

Summarization is conditional on measured history pressure. Preserve source references, timestamps, unresolved requests, and privacy classification. Treat summaries as derived, potentially lossy data; never let a summary mint tool permission, preserve an expired approval, change trusted instructions, or turn untrusted calendar text into authority. Re-fetch authoritative state before acting.

A new provider or model means recounting/rebudgeting. Session switches and cloud routes must not reuse another person's private context.

### Optimize only from evidence

Compare a small fixed set of representative runs: a direct status command, a tool-backed question, and a longer-history/calendar case. First reduce unnecessary model calls, irrelevant tool schemas, duplicate context, and oversized results. Then evaluate a bounded history window. Evaluate summarization only if its token/latency cost is recovered while task quality remains acceptable.

Defer vector databases, semantic caches, automatic prompt tuning, multi-agent handoffs, and multi-level memory until a measured requirement exists. Keep the earlier flow as the rollback baseline.

### Established context/token libraries and tools

Use established infrastructure, while keeping Jarvis's context-selection and authorization rules explicit. Tokenization, usage accounting, request budgeting, conversation memory, and observability solve different problems; no single dependency should silently own all of them.

| Option | Decision and phase | Suitability, cost, coupling, and learning |
|---|---|---|
| **Rig response usage and finish metadata** | First choice in 4B; already installed | No new service or tokenizer. Preserve provider availability/provenance when normalizing. Darren implements aggregation and stage accounting, not a replacement provider SDK. Rig is useful infrastructure, not a universal billing authority. |
| **llama.cpp tokenizer/template facilities** | First counting approach to investigate in 4C | Uses the deployed model; no duplicate tokenizer assets in Jarvis. Adds a local network call and still requires verified parity with actual tool/chat serialization. Keep behind LocalLlm. Learn exact versus estimated request accounting. |
| **Hugging Face tokenizers, Rust implementation** | Conditional 4C alternative if local/offline counting is useful | Established industry tokenization ecosystem, Apache-2.0. Rust fits the target, but verify selected release/features on Linux ARM64. Adds tokenizer files, memory, build cost and parity maintenance. Load the exact versioned tokenizer; verify it against llama.cpp before trusting counts. Current upstream is undergoing a v1 transition, so prefer a suitable stable release rather than copying a prerelease quickstart. No custom BPE/tokenizer implementation. [Project](https://github.com/huggingface/tokenizers) |
| **OpenAI tiktoken** | Optional Python evaluation tool for supported OpenAI model encodings, not the Gemma counter | Established MIT tokenizer with a Rust core and Python interface; verify selected ARM64 package/build. An OpenAI-compatible endpoint does not imply an OpenAI tokenizer. Rust ports are separately maintained choices, not automatically official SDKs. Keep this out of Jarvis unless the actual provider/model makes it appropriate. [Project](https://github.com/openai/tiktoken) |
| **tracing + tracing-subscriber** | Default visibility in 4B/7C | Established Rust ecosystem, suitable for lightweight structured local reports. Use bounded/filtered logging and no prompt contents by default. Darren learns spans and interprets latency/token evidence. Do not invent a telemetry framework. [tracing](https://github.com/tokio-rs/tracing) |
| **OpenTelemetry Rust + tracing-opentelemetry + OTLP export** | Optional 7D when cross-service or visual trace inspection is useful | OpenTelemetry is an industry standard, while the inspected Rust documentation labels its signals Beta; pin compatible crate versions. Export batches add buffers, network traffic and maintenance. Pi runs instrumentation; a collector/backend can live elsewhere. Keep export setup at App's infrastructure edge. Map token/model/stage fields to supported GenAI conventions and record schema versions instead of coupling policy to telemetry. [Rust status](https://opentelemetry.io/docs/languages/rust/), [tracing bridge](https://github.com/tokio-rs/tracing-opentelemetry), [GenAI conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/) |
| **Langfuse through its documented OpenTelemetry ingestion** | Optional visual backend in 7D; evaluate, do not mandate | Established LLM observability tooling for traces, usage and evaluation. It does not trim context or enforce tool limits. Verify attribute mapping from the Rust exporter; do not assume first-party Rust auto-instrumentation. Self-hosting adds multiple operational dependencies, so prefer a separate suitable host if chosen, not the inference Pi by default. Check selected edition/license and deployment resources. Hosted telemetry requires an explicit data policy; export redacted metadata by default. [OTel integration](https://langfuse.com/integrations/native/opentelemetry), [self-hosting](https://langfuse.com/self-hosting) |
| **SQLite FTS5** | Conditional after 7B, only if bounded keyword retrieval is useful | Mature SQLite extension avoids another database for searching user-permitted notes/history. Works with the selected ARM64 SQLite build if enabled. Indexes consume disk and need the same account filtering, retention and deletion as their sources. FTS tokenization is search indexing, not LLM token counting. Darren chooses relevance/selection policy. [FTS5](https://sqlite.org/fts5.html) |

Recommended adoption sequence: **existing Rig usage → local tracing report → model-appropriate counting and a small Jarvis budget function → bounded sessions → measured optimization → optional OpenTelemetry/Langfuse inspection**.

Use libraries for tokenization and telemetry, but keep a small deterministic context-selection function in Rust: only Jarvis knows which messages are authoritative, which tool exchanges must remain paired, and which household data may be sent. This is application logic worth learning, not unnecessary infrastructure reinvention.

Do not add LangChain/LangGraph, a separate memory platform, vector database, or prompt-compression model merely to get token counts. Revisit semantic retrieval or learned compression only after a concrete task defeats bounded history and simple retrieval.

## 6. Tapo camera integration

**Confirmed:** Tapo C100, already connected to the user's app. **Unconfirmed:** hardware revision, installed firmware, camera count, enabled local Camera Account, and a working stream from jarvis-ai. Existing app access does not itself establish third-party RTSP access.

The current C100 product specification lists RTSP/ONVIF and H.264/1080p video. Verify those against the installed hardware revision/firmware rather than applying every specification from a newer product revision. This makes local ingestion a credible path, not a completed integration. [Tapo C100 specifications](https://www.tp-link.com/us/home-networking/cloud-camera/tapo-c100/).

Tapo documents RTSP/ONVIF support for many wired models, with battery-model restrictions and exceptions. Verify the exact model and firmware first. Typical single-lens paths are stream1 and stream2; some dual-lens models use different paths. Use a separate local Camera Account where supported, not the Tapo cloud account. [Tapo setup](https://www.tp-link.com/us/support/faq/2680/), [compatibility and stream FAQ](https://www.tapo.com/en/faq/724/).

Start with one configured local RTSP stream. Use ONVIF only if discovery, profile selection, or another needed control justifies it. Do not reverse-engineer a Tapo cloud API merely to obtain local video. If the actual camera cannot expose a supported local feed, mark that ingestion path blocked and continue learning with a recorded clip; do not assume a hub resolves it.

Use a reserved LAN address and keep credentials out of logs, command lines where avoidable, Git, and model context. Keep RTSP/ONVIF inside the protected home network; do not port-forward it. Vendor recording/cloud services have their own privacy settings—Jarvis processing locally does not disable them.

Default workload:

~~~text
Tapo → RTSP/decoder → bounded latest-frame buffer
     → sampled detection/tracking → zone events
     → optional quality-gated face matching
     → authenticated structured event delivery to Rust
~~~

Prefer a lower-resolution stream for initial occupancy experiments. Facial detail may require the main stream; measure face size, blur, lighting, angle, and CPU cost first. Do not promise recognition from a distant or ceiling-facing camera. Share one ingestion pipeline where practical rather than opening another connection for every feature.

For PTZ cameras, a fixed image-space zone is invalid after the camera moves. Begin with a fixed view/preset; later associate calibration and zone version with the active preset, or suspend zone decisions while moving.

Disconnect means unknown/stale occupancy, not an empty room. Use tracker hysteresis and event freshness. Keep frame queues bounded; drop old frames for freshness rather than accumulating latency.

Before multiple cameras, measure concurrent LLM/CV load, decoding cost, memory, temperatures, and stream connection constraints on the actual hardware.

## 6A. Optional Jarvis-managed recording and private storage

The user's proposed cloud-storage role is feasible as **self-hosted recording with authenticated remote access**, subject to disk capacity, sustained I/O, network bandwidth, and operational reliability. This is the initial interpretation; a general-purpose family file-sync service or off-site cloud backup remains a separate decision.

Jarvis would pull and record the C100's supported local stream. Do not assume the camera can replace its Tapo Care upload destination with an arbitrary Jarvis server, or that recordings automatically appear inside the Tapo app.

Keep three responsibilities distinct:

~~~text
camera encoded stream → established recorder → media files on dedicated storage
                                   ↓
                       metadata/retention catalogue
                                   ↓
               Jarvis query/control + authenticated playback

same camera feed → perception → small observations → operational SQLite
~~~

Prefer recording encoded video without re-encoding when compatible. Decode only the perception path. Do not send frames or video through Assistant or put video blobs in the main SQLite database.

Evaluate an established recorder first:

- FFmpeg's segment muxer is a focused primitive for one-camera learning. It avoids implementing codecs/muxing; Darren still learns process supervision, segment lifecycle, retention and metadata. Segment boundaries depend on keyframes, so do not promise exact arbitrary clip cuts. Distribution/license depends on the selected FFmpeg build.
- Frigate is an option if the goal expands into an NVR with retention/playback. It supports an ARM64 deployment path, but check the exact Pi, decoding and accelerator support. It adds another service and database/configuration surface. Avoid running duplicate Frigate and custom C++ detection by default; choose which owns inference.
- Neither choice is an unconditional adoption. Start with one bounded recording trial and measure it. [FFmpeg segment recording](https://ffmpeg.org/ffmpeg-formats.html#segment_002c-stream_005fsegment_002c-ssegment), [Frigate recording](https://docs.frigate.video/configuration/record/), [installation](https://docs.frigate.video/frigate/installation/).

For retention planning, approximate decimal GB/day = measured average bitrate in Mbps × 10.8 × camera count. Example only: 2 Mbps for one camera is about 21.6 GB/day before overhead. The C100's actual bitrate is not established here. Add disk headroom and decide retention days/byte quota before continuous recording.

Prefer suitable dedicated SSD/HDD storage over filling the Pi's boot microSD. Disk capacity is separate from 8/16 GB RAM. Pick the recording host after measuring I/O contention with inference; do not automatically assign every workload to jarvis-ai.

Required behavior: bounded segment duration, retention/byte cap, minimum-free-space threshold, safe handling of incomplete segments, restart reconciliation, and explicit disk-full/missing-drive state. If an external mount disappears, do not silently write recordings into the boot filesystem underneath its mount point. Recording failure must not crash the assistant.

Keep raw recording disabled until this phase is deliberately undertaken with agreed retention. A home recorder is not an off-site backup: theft, disk failure, fire, power loss, and home-network outage still matter. Add encrypted backup only if that separate requirement is chosen.

For remote access, start with a maintained VPN such as WireGuard and authenticated playback, not publicly forwarded RTSP or an open media directory. VPN membership is network access, not per-person authorization. Follow device/user revocation and restrict clip access. [WireGuard](https://www.wireguard.com/).

Do not build a Dropbox replacement, object-storage cluster, custom transcoder, or custom cryptography to satisfy one camera's recordings.

## 7. Household facial recognition

Goal: optionally recognize Darren and explicitly enrolled family members for local contextual features. This is a separate milestone after dependable camera ingestion and anonymous occupancy.

Distinguish:

- Person detection: somebody is visible.
- Face detection: a usable face region exists.
- Face embedding: numeric representation produced by a model.
- Recognition: compare with enrolled templates and possibly reject.
- Authentication/authorization: trusted account and permission decisions, independent of face matching.

### Pipeline and ownership

~~~text
tracked person / face crop
 → quality gate and alignment
 → embedding
 → compare with small enrolled gallery
 → threshold + ambiguity margin + temporal consistency
 → candidate household identity OR unknown/ambiguous
 → Rust contextual observation
~~~

Keep detection, alignment, embeddings, and matching on jarvis-ai. Rust owns the explicit household person ID, enrollment/revocation policy, and permission mapping. Bind observations to camera, producer session, track ID, model version, gallery version, and time. Track IDs are not person IDs.

For a handful of family members, an in-memory matrix and exact comparisons are enough. No vector database or face-recognition agent is warranted.

Always permit unknown and ambiguous outcomes. Do not force every face to match a family member. Similarity is not a calibrated probability. Do not copy a tutorial threshold as a household accuracy guarantee.

### Enrollment and privacy

- Enroll each participating person deliberately, with their knowledge and agreement; handle children with appropriate guardian involvement.
- Start by enrolling Darren only; expand after held-out checks.
- Use a small set of varied, quality-controlled samples. Keep evaluation samples separate by capture session.
- No automatic guest enrollment or inference of sensitive traits.
- Treat face crops and embeddings as sensitive biometric data; embeddings are not anonymous.
- Store the minimum needed locally under restricted access. Keep identity names separate from numeric template records where practical.
- Default runtime crops to transient memory. Explicitly retained enrollment samples need a deletion policy.
- Revocation deletes templates and invalidates active galleries/caches on both nodes. A producer with obsolete enrollment state must not continue producing accepted identity matches.
- Retention/deletion includes backups and dataset exports; record their lifecycle.
- Never send face images, embeddings, or identified movement history to cloud models by default.

A face match may support a low-risk greeting. It must not unlock a door, approve a calendar write, reveal a private appointment, or authenticate the person speaking near the camera. Someone being visible does not establish who issued a request.

Do not assume a normal RTSP camera prevents photo/video replay. Anti-spoofing would require a separately evaluated capability; avoid depending on it for authorization.

### Model choice

Evaluate **OpenCV YuNet + SFace** first: official C++ examples exist, and it reuses the proposed OpenCV stack. Benchmark CPU inference on the AI Pi before adding another runtime. The inspected model directories use MIT for YuNet and Apache-2.0 for SFace; pin artifact checksums and retain the selected model's license.

Sources: [OpenCV face pipeline](https://docs.opencv.org/4.x/d0/dd4/tutorial_dnn_face.html), [YuNet license](https://raw.githubusercontent.com/opencv/opencv_zoo/main/models/face_detection_yunet/LICENSE), [SFace model and license](https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface).

Evaluate ONNX Runtime only if it materially improves the chosen model's deployment or performance. ONNX format does not establish Hailo compatibility. InsightFace is an alternative to investigate if the baseline fails, not an automatic upgrade: its code license and pretrained-model terms differ. [InsightFace licensing](https://github.com/deepinsight/insightface#license).

Versioned model upgrades may require re-enrollment or template recomputation; embeddings from different models are not automatically comparable.

## 8. Calendar tools and future integrations

Calendar provider is not yet confirmed. Support **one chosen provider first**. Google Calendar, Outlook/Microsoft Graph, and Apple/iCloud/CalDAV have different authentication and event semantics; do not build all adapters in advance.

An integration in the Codex app does not automatically supply credentials or a runtime library to Jarvis.

### First capability: bounded read

Tool concept: calendar_list_events for one authorized calendar and bounded time interval.

~~~text
explicit command or validated tool proposal
 → trusted caller/calendar allowlist
 → Calendar operation
 → provider adapter
 → normalized events + freshness/truncation
~~~

Choose an explicit provisional horizon (for example, at most seven days) and result limit (for example, 50); document these as application policy rather than provider limits. Return an explicit empty result, partial-result marker, stale cache state, or provider failure—never conflate them.

Model arguments must not select arbitrary account credentials. Associate calendar access with an authenticated caller or deliberately shared household calendar. Facial recognition cannot select a private account on someone's behalf.

Preserve:

- Provider/account/calendar/event identifiers.
- Timed events as instants plus source timezone.
- All-day events as dates with exclusive end dates, not midnight UTC appointments.
- Recurring occurrence identity and exceptions.
- Cancellation state and fetch time.

Use Asia/Singapore as the user's initial display default, not as a replacement for every event's source timezone. Resolve ambiguous natural-language dates before writes.

Prefer provider expansion of recurrences for the requested window. Do not implement recurrence rules yourself. Event descriptions and attachments are untrusted content, not tool instructions. Send only fields needed to answer the question.

### Second capability: an approved write

Separate event creation from reading:

~~~text
proposal → deterministic validation → concrete preview
 → explicit authenticated approval
 → one provider write → authoritative result → audit
~~~

Bind approval to the normalized event contents, calendar, caller, and expiration. Any material edit invalidates the old approval. Initially create one ordinary timed event on one calendar, without attendees or notifications to others.

Use provider-supported duplicate-prevention mechanisms or an operation ID with reconciliation. A lost response does not prove creation failed; do not blindly retry and create duplicates. Google permits client-generated event IDs; Microsoft Graph exposes transactionId for creation retry scenarios. [Google creation](https://developers.google.com/workspace/calendar/api/guides/create-events), [Graph event resource](https://learn.microsoft.com/en-us/graph/api/resources/event?view=graph-rest-1.0).

Attendee invitations, deletion, recurring-series edits, shared-family writes, and autonomous rescheduling are later capabilities with separate permissions and preview semantics.

### Authentication and lifecycle

Use the chosen provider's supported authorization flow, minimum scopes, and a maintained OAuth library where applicable. Never hand-roll token signing or credential protocols. Keep refresh tokens outside prompts/logs; restrict storage and support revocation.

Start with bounded on-demand reads. Add persistent sync only after it solves a real need. When introduced, support pagination, deletions, expired sync state, and full resync. Do not treat a cached calendar as current during outages.

Sources: [Google event semantics](https://developers.google.com/workspace/calendar/api/concepts/events-calendars), [incremental sync](https://developers.google.com/workspace/calendar/api/guides/sync), [Graph calendar view](https://learn.microsoft.com/en-us/graph/api/calendar-list-calendarview?view=graph-rest-1.0), [Rust oauth2](https://docs.rs/oauth2/latest/oauth2/).

Calendar access can legitimately contact a cloud calendar provider while cloud LLM use remains disabled. These are separate outbound-data permissions. Never upload camera observations to a calendar provider as an incidental side effect.

Future integrations follow the same pattern: one concrete operation, trusted context, bounded adapter, typed result. A registry trait or integration framework must earn its cost through actual repetition.

## 9. State, concurrency, persistence, and errors

| State | Owner / lifetime |
|---|---|
| Messages, deadlines, tool budget | Request-local run |
| Token/latency counters and context decisions | Per-request/run metadata; optionally persisted with retention |
| Session history and summaries | Bounded, session/account-scoped context; optional retention, never permissions |
| Clients | Long-lived reusable dependencies |
| Tracker and transient face crops | C++ process, scoped per camera/session |
| Small enrolled gallery | Local perception memory loaded from versioned enrollment data |
| Household IDs and enrollment status | Rust-owned policy data; no automatic account authority |
| Observations and action audit | SQLite on application node |
| Calendar cache | Optional, account-scoped, explicitly stale when outdated |
| Secrets | Restricted credential storage, never ordinary event records |
| Training datasets | Explicit versioned exports |
| Model artifacts | Versioned manifests with checksum/preprocessing/runtime |
| Raw media | Off initially; separate bounded store only if required |
| Recording catalogue | Metadata referencing files on verified mounted storage; separate from video bytes |

Begin with one interactive run at a time. Before concurrency, define bounded queues, admission, stop behavior, task ownership, and failure reporting. Avoid Arc<Mutex<App>>. Prefer single owners and channels; use Arc only for actual shared ownership.

Frame overflow may discard stale frames. Command overflow must reject visibly, not silently drop a requested action. Do not hold locks across network awaits. Keep database blocking work and inference off Tokio executor threads.

Use a dedicated worker for repeated SQLite operations. spawn_blocking is for bounded blocking work, not a magical cancellation mechanism. [Tokio blocking work](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).

Supervise long-running tasks and join them on shutdown. A cancellation signal is not proof that remote work or physical actuation stopped. [Tokio shutdown](https://tokio.rs/tokio/topics/shutdown).

SQLite stays on local storage with migrations, constraints, parameterized queries, and retention. WAL still has one writer at a time and should not be shared over a network filesystem. [SQLite WAL](https://sqlite.org/wal.html).

Introduce small error types only as needed: timeout/unavailable, invalid model turn, denied proposal, tool failure, budget exhausted; later storage, camera, calendar authorization, and uncertain-write errors. Preserve source causes internally and sanitize user-facing diagnostics.

## 10. Event delivery, ML, audio, cloud, and improvement

### Observation delivery

Begin with authenticated HTTP POST of small event batches. Use event ID, schema version, producer session, sequence number, capture/receipt times, camera ID, zone version, and model version. Scope track IDs to camera/session. Reject unsupported schemas and expire stale observations.

Use deduplication on retries. A durable acknowledgement follows a committed write. For initial noncritical occupancy, explicitly report gaps and unknown state during outages. Add a bounded durable sender spool only when guaranteed recovery is required. No message broker initially.

### ML and deployment

Operational events → versioned export → offline Python features/evaluation → artifact → shadow inference → approved promotion.

Begin with anonymous occupancy patterns. Person-specific routine learning is a separate opt-in scope; recognition does not automatically authorize long-term individual movement profiles.

Use temporal/session splits, meaningful negative examples, and a simple baseline. Avoid random splitting adjacent frames/windows. Keep preprocessing, feature order, labels, model weights, checksum, dataset version, and runtime requirements in the artifact manifest. Preserve a previous working artifact.

Do not put training in the production Rust process. Python experimentation, C++ inference, and Rust policy remain independent learning paths.

### Audio and UI

Push-to-talk → bounded audio → STT → ordinary request path. Response → TTS → bounded playback. No audio agents. Wake words, VAD, speaker recognition, full duplex, and echo cancellation come later.

Slint/API are presentation adapters. Respect UI event-loop ownership and keep blocking inference off callbacks. Do not read private calendar content aloud merely because a camera recognized someone; consider authenticated request context and shared-room disclosure.

### Cloud models

Route after privacy classification, caller authority, and deterministic-command checks. Local remains default. Check the entire context before every cloud request, including newly acquired calendar/tool data. Pin a run to one provider initially.

No silent cloud fallback. OpenRouter routing/ZDR controls supplement Jarvis policy; they do not replace it. [Provider routing](https://openrouter.ai/docs/guides/routing/provider-selection), [ZDR](https://openrouter.ai/docs/guides/features/zdr).

### Controlled improvement

Use offline proposal → isolated evaluation → report → explicit promotion → monitoring/rollback. Begin with a prompt or threshold. Normal runtime credentials must not permit source edits, deployments, or gallery/model replacement. Safety-relevant configuration deserves the same care as code.

## 11. Dependency decisions

Current [Cargo.toml](/Users/darrenng/Desktop/Desktop/personalProjects/jarvis/Cargo.toml): dotenvy 0.15.7; reqwest 0.13.5 with JSON/defaults; rig-core exactly 0.42.0; Tokio 1.53.1 with full features. The inspected lockfile resolves these versions.

Keep these during the first slices. Narrow Tokio/features only when beneficial. Rig and the app share reqwest 0.13.5; there is no duplicate-version problem. Feature unification and TLS/native build dependencies affect Pi build cost. Target-specific packages in Cargo.lock are not proof of Pi runtime bloat.

Do not upgrade Rig simply to match current upstream examples. Use [version-matched documentation](https://docs.rs/rig-core/0.42.0/rig_core/). Pin future dependencies when their phase is reached, after checking selected versions, licenses, ARM64 packages, and the actual device.

| Dependency / timing | Problem, maturity, platform and cost | Boundary and learning retained |
|---|---|---|
| Tokio + reqwest / now | Established Rust scheduling/networking; Linux ARM64 suitable. Reuse runtime/TLS/pooling. Bound tasks/buffers; native crypto affects build time. MIT / MIT-or-Apache-2.0 in inspected versions. | Direct infrastructure. Darren owns budgets, cancellation, routing and errors. |
| Rig / now | Working provider integration; evolving MIT project. Avoid protocol reimplementation; upgrades need deliberate review. Model memory stays on AI node. | Model adapter only. Darren builds bounded execution and policy. |
| Serde + serde_json / tools | Mature serialization, modest footprint; avoid custom JSON parsing. Limit input size and allocations. | Typed contracts. Darren defines semantics, units, validation. [Serde](https://serde.rs/) |
| Schemars / conditional | Avoid duplicated schema definitions when useful; not runtime authorization. One empty-argument tool does not require it. | Schema generation at edge. [Schemars](https://docs.rs/schemars/latest/schemars/) |
| thiserror + tracing / loop | Established error/diagnostic tools; small overhead unless logs are excessive. | Direct use. Darren chooses categories, fields, redaction, audit distinction. [thiserror](https://docs.rs/thiserror/latest/thiserror/), [tracing](https://github.com/tokio-rs/tracing) |
| sysinfo / status | Portable OS readings on development/target systems; selective refresh avoids needless process scans. | Status tool only. Darren defines observation contract. [sysinfo](https://docs.rs/sysinfo/latest/sysinfo/) |
| SQLite + rusqlite / persistence | Mature embedded storage, good ARM64 fit; synchronous access needs isolation, SD write volume matters. | Small concrete storage module. Learn SQL, transactions, migrations, retention. [rusqlite](https://github.com/rusqlite/rusqlite) |
| SQLx / alternative | Evaluate if async integration/query tooling simplifies the real application; avoid unnecessary pools/build complexity. | Choose instead of layering over rusqlite. [SQLx](https://docs.rs/sqlx/latest/sqlx/) |
| Axum / event ingress or API | Established Tokio HTTP routing; ARM64 suitable; body/concurrency limits required. | Transport only. Darren implements auth and acceptance semantics. [Axum](https://github.com/tokio-rs/axum) |
| reqwest + oauth2 / calendar | Reuse HTTP and OAuth machinery; token lifecycle remains application responsibility. Linux ARM64 practical; provider quotas are external. | One concrete calendar adapter. Learn scopes, expiry, timezones, recurrence, idempotency. No all-provider SDK framework. [oauth2](https://docs.rs/oauth2/latest/oauth2/) |
| OpenCV + GStreamer / perception | Mature CV/media stack; Linux ARM64 appropriate, codecs and frame copies dominate resource cost. OpenCV 4.5+ Apache-2.0; GStreamer LGPL plus plugin-specific terms. | C++ process. Learn images, timing, zones, tracking and bounded queues. [OpenCV license](https://opencv.org/license/), [GStreamer appsink](https://gstreamer.freedesktop.org/documentation/app/appsink.html), [licensing](https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html) |
| YuNet + SFace / recognition | Official OpenCV examples, small-stack starting candidate, CPU/ARM64 benchmark required. No automatic HAT support. Model licenses differ; see section 7. | Local perception. Learn alignment, embeddings, unknown rejection, calibration and evaluation. |
| libcurl + nlohmann/json / C++ events | Mature portable transport/parsing; avoid writing HTTP/JSON. Header compilation and buffers have costs; messages are small. | C++ transport edge. Learn event IDs, retry bounds and acknowledgements. [libcurl](https://curl.se/libcurl/), [JSON](https://github.com/nlohmann/json) |
| ONNX Runtime / model-dependent | Mature MIT runtime with ARM64 build support; binaries/session memory add deployment cost. Standard providers do not establish Hailo compatibility. | Inference process only. Learn parity and preprocessing. [ARM builds](https://onnxruntime.ai/docs/build/inferencing.html), [providers](https://onnxruntime.ai/docs/execution-providers/) |
| HailoRT / exact board only | Vendor-supported accelerator path; firmware, compiler, runtime, model and board family must match. Compiler host requirements may differ from Pi runtime. | Vendor adapter in perception. Learn quantization and throughput. Verify selected-stack licenses. [HailoRT](https://github.com/hailo-ai/hailort), [Pi AI HATs](https://www.raspberrypi.com/documentation/accessories/ai-hat-plus.html) |
| scikit-learn, numerical/data tools; PyTorch when needed / offline | Industry-relevant ML ecosystems; small classical experiments can run on Pi, heavy training belongs elsewhere. Python environments and numerical threads require management. | Offline experiments. Learn datasets, features, baselines, splits and metrics. [scikit-learn](https://scikit-learn.org/stable/modules/cross_validation.html), [PyTorch](https://github.com/pytorch/pytorch) |
| Eigen / optional; LibTorch / defer | Eigen useful for a focused C++ algorithm exercise. LibTorch only for an actual PyTorch-specific deployment need; larger native packaging burden. Verify exact ARM64 distribution. | Keep learning experiments isolated. No duplicate inference stacks just to use C++. [Eigen](https://libeigen.gitlab.io/eigen/docs-nightly/), [LibTorch](https://docs.pytorch.org/tutorials/advanced/cpp_frontend.html) |
| Whisper.cpp / STT | Established native project with Pi/ARM support; model size and concurrency set latency. | Audio process adapter. Learn capture, transcription evaluation and interruption. [Whisper.cpp](https://github.com/ggml-org/whisper.cpp) |
| Piper or sherpa-onnx / TTS | Piper current repo is GPL-3.0 and seeks maintainers; sherpa-onnx is Apache-2.0 with Linux ARM64 support. Check selected voice/model terms separately. | Audio adapter. Learn playback, queue limits and quality/latency. [Piper](https://github.com/OHF-Voice/piper1-gpl), [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) |
| CPAL; Slint / interfaces | Audio I/O and native UI; Linux backend/build dependencies and Slint embedded licensing require evaluation. | Presentation edges. Learn buffers/event loops rather than implementing audio drivers or a UI toolkit. [CPAL](https://github.com/RustAudio/cpal), [Slint](https://docs.slint.dev/latest/docs/rust/slint/), [licensing](https://slint.dev/pricing) |
| Existing Rig usage + deployed llama.cpp tokenizer / context | Reuse existing metadata and model-specific counting before adding another tokenizer dependency. Tool/template overhead and absent provider metrics need explicit treatment; counting itself has latency. | Model adapter exposes measurements; context module owns budget/selection; Darren learns accounting and flow tradeoffs. See section 5A. |
| FFmpeg or Frigate / optional recording | Mature media primitives versus an integrated NVR; ARM64 packaging and actual Pi decode/inference paths must be verified. Storage and I/O dominate, and licenses depend on the selected components/build. | Separate recording process; Jarvis controls a narrow catalogue/API. Darren learns lifecycle, retention and failure isolation. See section 6A. |
| WireGuard / optional remote viewing | Established encrypted network tunnel with Linux/ARM deployment; avoids homemade transport security. Adds device/key administration and does not supply playback authorization or backup. | Deployment boundary, not Assistant dependency. Darren learns network access and revocation. See section 6A. |

Except for the exact vendor path, none of these recommendations implies AI HAT acceleration. Do not add every library at once. Recheck exact artifact licenses and package availability at adoption; this is a selection plan, not a completed distribution/license audit.

## 12. Minimal verification policy — boundary values first

The user's preference is minimal meaningful testing. Use the smallest checks that distinguish correct behavior at the new boundary.

- Normally one successful observable run plus one compact table-driven boundary test for the changed policy.
- Numeric limits: just below, exactly at, and just above the relevant threshold; omit redundant cases when they exercise identical logic.
- Shape/authorization boundaries: one representative valid and invalid case for each distinct rejection rule.
- One target-Pi smoke check per new external integration. Reuse existing fixtures; no broad repeated suite unless a new failure warrants it.
- No tests for trivial pass-through methods or reversible documentation edits.
- Do not exhaustively cross-product every input.
- Use built-in Rust tests first. Test orchestration using a scripted model, not probabilistic live-model wording.

The small table below is a menu used when each feature exists, not a test suite to build immediately.

| Boundary | Minimum cases | Required result |
|---|---|---|
| Completion deadline D | Completion before D; permanently pending future until D | Success / timeout; use deterministic time control if needed, not flaky exact network timing |
| Input token budget B | Assembled request B-1, B, B+1 tokens | Fit / fit / trim allowed content or reject before generation |
| Output reserve | Input plus output reserve fits exactly; exceeds by one | Preserve reserve / reject or trim, never rely on silent server truncation |
| Usage accounting | Known two-request fixture; one missing usage report | Correct cumulative count / labelled partial total, not false zero |
| History trimming | Whole completed exchange; pending tool exchange at budget boundary | Trim whole eligible exchange; retain matching active call/result |
| Context isolation | Same session; another user's session | Reuse only authorized session content |
| Compaction correctness | One fixed conversation with a current fact, expired approval and untrusted instruction | Keep task-relevant data; no revived authority or promoted instruction |
| Tool call budget 1 | 0, 1, 2 proposed calls | Plain answer / one execution / rejection without execution |
| Model request budget 2 | Final response by request 2; proposal requiring request 3 | Finish / stop before request 3 |
| Tool arguments | Empty object; extra field; wrong shape | Accept only the declared shape |
| Tool name/permission | Known allowed; unknown; known denied | Only allowed call executes |
| Argument size B | B and B+1 bytes | Accept size boundary / reject before processing |
| Queue capacity Q | Q and Q+1 items | Remain bounded; documented drop/reject policy |
| Recording free-space floor F | F+epsilon, F, F-epsilon | Follow documented stop/purge threshold; never fill reserved space |
| Recording mount/access | Mounted disk; missing disk; unauthorized clip request | Record safely / stop visibly / deny playback |
| Observation freshness T | T-epsilon, T, T+epsilon | Document whether equality is stale; use the same rule everywhere |
| Calendar interval | start < end; start = end; start > end; horizon H and H+epsilon | Only positive intervals within horizon accepted |
| Calendar results N | 0; N; N+1 available | Empty distinguished from failure; bounded results and explicit continuation/truncation |
| Event boundaries | Event ends at window start; starts at window end; one all-day case | Consistent overlap/exclusive-end semantics |
| Write approval | Valid matching approval; expired approval; edited draft | Exactly the approved content may execute |
| Calendar write uncertainty | Simulated accepted write with lost response | Reconcile or return uncertain; no blind duplicate |
| Face acceptance | Best score tau-epsilon, tau, tau+epsilon; ambiguity margin at/around delta | Defined equality behavior; ambiguous match remains unknown |
| Face quality/enrollment | Unusable crop; revoked person; empty gallery | No accepted identity |
| Revocation/version | Current gallery; stale gallery after revocation | Stale identity result not accepted |
| Physical permission | Allowed; denied; uncertain previous attempt | Execute / no side effect / reconcile first |

For faces, a threshold unit test proves only matching logic. Add **one small held-out session evaluation** with enrolled people plus consenting non-enrolled examples and representative lighting/angles. Report false matches and false nonmatches; do not invent an accuracy claim from three test scores. If the small evaluation is inconclusive, keep recognition in shadow mode. This is minimal model validation, not a large testing campaign.

For occupancy ML, use one reproducible temporal split and one simple baseline. For CV, use one short annotated clip. Do not launch broad benchmark hunts.

## 13. Incremental roadmap

Every phase below specifies goal/reason, architecture/responsibilities/direction, existing and potential files, learning/library choices, minimum verification and exit, mistakes, and what to defer.

Phases 1–7, including 4B/4C, are the shared foundation. Add 7B when multi-turn sessions begin; 7C is a measured optimization exercise, not a blocker for every feature. After the foundation, calendar, camera, voice, cloud, and device branches can progress independently. Recognition depends on useful camera imagery; recording does not depend on recognition or ML.

### Phase 1 — Bound the existing completion [NOW]

- **Goal/why:** prevent indefinite waiting in the already-working path.
- **Architecture:** Assistant → LocalLlm → Rig; LocalLlm owns the request deadline.
- **Files:** existing LocalLlm only initially; no new module.
- **Learn/libraries:** futures, nested Results, ?; existing Tokio deadline facility.
- **Minimum verification/exit:** one normal Pi completion and one deterministic pending-request timeout. Deadline does not break normal output.
- **Mistakes/defer:** do not blindly apply the three-second health budget; no retries, traits, tools, provider changes, or generic error hierarchy.

### Phase 2 — Observe system status directly [NOW]

- **Goal/why:** establish a real observation before model orchestration.
- **Architecture:** App test harness → deterministic status function → OS readings; return typed values.
- **Files:** modify App and main module declaration; add tools/mod.rs and tools/system_status.rs.
- **Learn/libraries:** structs, units, borrowing, serialization; evaluate selective sysinfo refresh.
- **Minimum verification/exit:** one host comparison plus unavailable-value representation. Status identifies the correct node.
- **Mistakes/defer:** no shell, process inventory, remote-node polling, registry trait, or agent loop.

### Phase 3 — Preserve one structured proposal [NOW]

- **Goal/why:** prove deployed function-calling compatibility without executing anything.
- **Architecture:** LocalLlm translates provider output into a small turn; Assistant inspects it.
- **Files:** LocalLlm, Assistant, App output and manifest; add model.rs.
- **Learn/libraries:** enums, vectors, moves, matching, correlation IDs; Rig and Serde.
- **Minimum verification/exit:** compact fixtures for text-only, call-only and mixed turns; one live proposal preserves name, arguments and ID.
- **Mistakes/defer:** no prose command parsing, automatic execution, broad provider abstraction, or model hunt.

### Phase 4 — One bounded tool round trip [NOW]

- **Goal/why:** establish trusted proposal → validation → execution → observation → answer.
- **Architecture:** Assistant owns run state; tools own allowlisted validation/dispatch; LocalLlm owns protocol continuity.
- **Files:** Assistant, LocalLlm, model, tools, App diagnostics; add a narrow scripted-model seam only if it earns its cost.
- **Learn/libraries:** state transitions, counters, focused errors; thiserror/tracing if helpful.
- **Minimum verification/exit:** one Pi happy path and a table for 0/1/2 calls, invalid name/arguments, and attempted third model request. Rejected input causes no execution.
- **Mistakes/defer:** no recursion, repair, retries, parallel calls, persistent conversation, or physical actions.

### Phase 4B — Observe tokens and execution stages [SOON; depends on 4]

- **Goal/why:** see where context, calls, and time are spent before optimizing.
- **Architecture:** model adapter preserves usage/finish metadata → run accumulator → compact CLI report. No raw prompt logging.
- **Files:** LocalLlm, model response contract, Assistant, App output; reuse tracing. No separate dashboard yet.
- **Learn/libraries:** measured versus estimated values, per-request versus cumulative usage, optional metadata; existing Rig/Tokio/tracing.
- **Minimum verification/exit:** one known two-request accounting fixture and one missing-usage fixture; one live run shows stages, calls, tokens and timing with honest availability labels.
- **Mistakes/defer:** no double-counted cache/reasoning subtotals, missing-as-zero metrics, broad telemetry stack, or hidden-reasoning logging.

### Phase 4C — Enforce a request context budget [SOON; depends on 4B]

- **Goal/why:** prevent overflow while preserving the instructions, current request, and active tool exchange.
- **Architecture:** Assistant → deterministic context assembly → provider adapter. Provider-specific counting remains at the adapter boundary.
- **Files:** Assistant, LocalLlm, configuration and model request contract; new context.rs.
- **Learn/libraries:** token versus byte counts, output reservation, template overhead, structural trimming; deployed llama.cpp tokenizer/template support where verified.
- **Minimum verification/exit:** B-1/B/B+1 input fixtures with reserved output, plus one active call/result fixture. Oversized required input fails clearly; actual requests report their selected budget.
- **Mistakes/defer:** no generic tokenizer assumed exact for Gemma, dropping trusted instructions, orphaned tool results, or automatic summarization.

### Phase 5 — One deterministic route [SOON]

- **Goal/why:** explicit status works with the model offline.
- **Architecture:** request router → direct status or Assistant; both reuse the same operation.
- **Files:** App input; new routing.rs.
- **Learn/libraries:** command enums and separation of interpretation/execution; no framework.
- **Minimum verification/exit:** direct status while AI node is unavailable; conversational request still follows bounded flow.
- **Mistakes/defer:** no LLM router, universal intent classifier, or generic workflows.

### Phase 6 — One supervised background health worker [SOON]

- **Goal/why:** learn lifecycle without making every operation depend on inference readiness.
- **Architecture:** App → supervisor → health task; explicit shutdown and degraded state.
- **Files:** App, Assistant health delegation, configuration; new runtime.rs.
- **Learn/libraries:** task ownership, bounded channels/watch state, cancellation; Tokio, optionally tokio-util.
- **Minimum verification/exit:** one offline-to-online transition and stop during pending work; no detached task remains.
- **Mistakes/defer:** no unbounded spawning, global mutex, or scheduling framework. Validate config rather than silently ignoring malformed values.

### Phase 7 — Persist one useful record [SOON]

- **Goal/why:** retain run outcome across restart and learn durable state.
- **Architecture:** run completion → storage owner → SQLite transaction.
- **Files:** App/Assistant completion handling and manifest; new storage.rs and initial migration.
- **Learn/libraries:** SQL, constraints, transactions, retention, blocking isolation; rusqlite.
- **Minimum verification/exit:** write/restart/read smoke check and duplicate-ID boundary; failures are explicit.
- **Mistakes/defer:** no ORM hierarchy, vector database, network-shared SQLite, or exactly-once claims.

### Phase 7B — One bounded multi-turn session [SOON when sessions begin; depends on 4C and 5]

- **Goal/why:** learn useful continuity without unlimited history or cross-user leakage.
- **Architecture:** session owner → bounded completed exchanges → context selection → Assistant; optional storage after phase 7.
- **Files:** context, Assistant, App/session entry; storage only if explicitly retaining history across restart.
- **Learn/libraries:** ownership/lifetimes, session IDs, privacy/freshness, complete-exchange trimming; existing collections and serialization.
- **Minimum verification/exit:** at/over history budget, pending-tool preservation, and different-session isolation. One short multi-turn conversation works within the cap.
- **Mistakes/defer:** no global chat history, automatic long-term memory, stale tool observations treated as current facts, or summary-based approvals.

### Phase 7C — Measure one agent-flow improvement [SOON after 4B/4C/5; summarization depends on 7B]

- **Goal/why:** learn to optimize from observed tokens, latency and task quality.
- **Architecture:** fixed scenario set → baseline run report → one change → comparison; retain rollback baseline.
- **Files:** a small experiment/report under experiments; change only routing/context/Assistant behavior relevant to the measured improvement.
- **Learn/libraries:** controlled comparison, context attribution, time/token tradeoffs; existing telemetry. No benchmark framework required.
- **Minimum verification/exit:** compare direct status, one tool question, and one bounded history case. Keep a change only if the targeted metric improves without wrong observations or broken policy; a small repeat is allowed if timing noise changes the conclusion.
- **Mistakes/defer:** no simultaneous prompt/model/runtime changes, token savings equated with correctness, vector memory, or semantic caching. Evaluate summaries only after trimming is insufficient, count their cost, and preserve provenance.

### Phase 7D — Inspect one exported agent trace [OPTIONAL LATER; depends on 4B and a real inspection need]

- **Goal/why:** see model/tool spans and token usage visually without building a dashboard.
- **Architecture:** existing tracing spans → bounded OpenTelemetry exporter → chosen backend; telemetry is outside the execution authority path.
- **Files:** App diagnostics setup and manifest; deployment configuration for exporter/backend. No changes to tool permissions.
- **Learn/libraries:** trace/span correlation, usage attributes, redaction, bounded export and version compatibility; OpenTelemetry/tracing-opentelemetry, optionally Langfuse via documented OTLP ingestion.
- **Minimum verification/exit:** one two-request tool run is correlated correctly with its token totals; one unavailable-exporter case leaves the run functional; verify private content is absent.
- **Mistakes/defer:** no mandatory telemetry cloud, raw prompt/biometric export, dashboard framework, or storage-heavy observability stack on the inference Pi by default.

### Phase 8A — Read one calendar [LATER; depends on 4–5 and 4C]

- **Goal/why:** add a useful second tool and learn authenticated external integration.
- **Architecture:** caller policy → Calendar operation → one provider adapter; same operation callable directly.
- **Files:** App/config/tools dispatch; new calendar/mod.rs and tools/calendar.rs. Storage only if tokens/cache need it.
- **Learn/libraries:** bounded queries, OAuth lifecycle, timezones, all-day/recurring events; reqwest, Serde, oauth2 where applicable, existing date/time tooling or one evaluated crate.
- **Minimum verification/exit:** one real read, compact interval/result-limit cases, and denied-calendar access. Empty, unavailable and partial results remain distinct.
- **Mistakes/defer:** no writes, invites, all-provider abstraction, background sync, or face-selected private calendar.

### Phase 8B — Create one approved calendar event [LATER; depends on 7 and 8A]

- **Goal/why:** learn controlled external side effects.
- **Architecture:** proposal → normalized draft → authenticated approval bound to content/expiry → provider write → audit.
- **Files:** calendar, tools dispatch, interface preview, storage; small approval record colocated with this operation.
- **Learn/libraries:** idempotency, validation, uncertainty, temporal semantics; reuse selected provider stack.
- **Minimum verification/exit:** one explicitly approved test event; valid/expired/changed approval cases; simulate lost write response without creating a duplicate.
- **Mistakes/defer:** no attendees, notifications, deletion, recurring-series edits, or autonomous scheduling. Camera identity never substitutes for approval.

### Phase 9 — Explicitly permitted cloud request [LATER; depends on 4–5 and 4B/4C]

- **Goal/why:** prove second-model substitution with privacy preserved.
- **Architecture:** routing eligibility → chosen provider → existing bounded run.
- **Files:** existing cloud placeholder, clients declaration, configuration, routing and model seam.
- **Learn/libraries:** enum versus trait, capabilities, secrets, cost bounds; Rig OpenRouter/compatible adapter evaluation.
- **Minimum verification/exit:** permitted public request succeeds; private tool context and denied route produce no cloud request.
- **Mistakes/defer:** no silent fallback, mid-run provider switching, camera/calendar context upload by default.

### Phase 10A — Read one sensor [LATER; depends on 6]

- **Goal/why:** learn real device integration safely.
- **Architecture:** supervised device adapter → typed timestamped measurement → application operation.
- **Files:** App/runtime, optional storage; one devices module.
- **Learn/libraries:** protocol/resource ownership, units, freshness; select maintained library for actual device.
- **Minimum verification/exit:** one valid reading and freshness at/after expiry or disconnection.
- **Mistakes/defer:** no universal driver framework, actuation, or LLM polling decisions.

### Phase 10B — One low-risk physical action [LATER; depends on 7 and 10A]

- **Goal/why:** learn permission and recovery with an indicator or similarly low-risk device.
- **Architecture:** trusted command → policy → current-state check → driver → observed result/audit.
- **Files:** device adapter, tools if needed, storage; focused action handling.
- **Learn/libraries:** idempotency, uncertain outcomes, reconciliation; reuse device stack.
- **Minimum verification/exit:** allowed/denied action and restart after uncertain attempt; denial has no side effect.
- **Mistakes/defer:** no locks, heaters, mains control, face-authorized actions, or blind retries.

### Phase 11A — One observation from a recorded clip [LATER; independent branch]

- **Goal/why:** learn C++/CV without camera/network variability.
- **Architecture:** separate C++ executable → preprocessing/detection → one zone observation.
- **Files:** new perception project; existing Rust unchanged.
- **Learn/libraries:** RAII, image layout, coordinate transforms, confidence; OpenCV and one inference backend.
- **Minimum verification/exit:** one short annotated clip including empty scene and a zone crossing; measure target-Pi latency/memory once.
- **Mistakes/defer:** no face matching, multi-camera identity, custom runtime, or Rust/C++ FFI.

### Phase 11B — One verified Tapo C100 live stream [LATER; depends on 11A]

- **Goal/why:** connect the already-owned, app-connected C100 through its local stream and establish usable image quality.
- **Architecture:** configured local stream → decoder → bounded latest-frame queue → existing perception.
- **Files:** perception ingestion/config; deployment camera inventory only. Existing Rust unchanged.
- **Learn/libraries:** RTSP/codec behavior, timestamps, stale frames, buffering; GStreamer/OpenCV.
- **Minimum verification/exit:** one live view and disconnect/reconnect; Q/Q+1 queue behavior remains bounded. Record face-image feasibility without implementing recognition.
- **Mistakes/defer:** no assumption every Tapo supports RTSP, no cloud reverse engineering, PTZ automation, multiple duplicate streams, or recording archive.

### Phase 11C — Deliver live observations to Rust [LATER; depends on 6–7 and 11B]

- **Goal/why:** learn a reliable small cross-node data boundary.
- **Architecture:** C++ consolidation → authenticated HTTP → Rust validation/dedup/storage/freshness.
- **Files:** perception transport, App/runtime/storage; new observations and HTTP ingress.
- **Learn/libraries:** sessions, IDs, retries, hysteresis, freshness; libcurl/JSON and Axum.
- **Minimum verification/exit:** duplicate event stored once; freshness boundary becomes unknown; queue overflow follows its declared policy.
- **Mistakes/defer:** no broker, raw-media retention, permanent occupancy after disconnect, or stale zones after camera movement.

### Phase 11D — One bounded local recording [OPTIONAL LATER; depends on 6 and 11B, not recognition]

- **Goal/why:** establish the first useful private-storage capability without building an NVR from scratch.
- **Architecture:** existing recorder process → verified dedicated media mount → bounded segments; Jarvis records metadata/status. Recording and inference are independent consumers.
- **Files:** deployment recorder configuration; new media adapter/catalogue only if needed; App/runtime/storage for status and metadata. No video in Git or operational DB blobs.
- **Learn/libraries:** stream copy versus decoding, bitrate/disk sizing, mount identity, retention, process supervision; evaluate FFmpeg first and Frigate if NVR features justify it.
- **Minimum verification/exit:** record/play one short segment; free-space floor boundary and missing-mount case. Retention bounds storage and failures leave the assistant running.
- **Mistakes/defer:** no indefinite recording, writing to the boot disk after mount loss, redundant detectors, exact-keyframe-independent cut promises, or assumed Tapo-app playback integration.

### Phase 11E — Authenticated private remote playback [OPTIONAL LATER; depends on 11D]

- **Goal/why:** make self-hosted recordings accessible away from home while keeping access controlled.
- **Architecture:** maintained VPN → authenticated existing playback interface or narrow Jarvis media query → allowed recordings. No public RTSP.
- **Files:** deployment/network access configuration; media/interface adapter only if the recorder's interface is insufficient.
- **Learn/libraries:** device keys, user access, revocation, uplink limits and outage behavior; WireGuard and selected recorder.
- **Minimum verification/exit:** one authorized remote playback, one denied/revoked user or device, and unavailable-home-server state. No unauthenticated recording URL.
- **Mistakes/defer:** no public port forwarding to camera, file-sync product, distributed object store, off-site-backup claims, or custom encryption.

### Phase 12A — Enroll Darren and evaluate offline matching [LATER; depends on 11A and usable imagery]

- **Goal/why:** learn embeddings and unknown rejection without live identity claims.
- **Architecture:** deliberate enrollment → versioned local gallery → offline face matcher → candidate/unknown.
- **Files:** perception face module and offline evaluation data manifest; household enrollment mapping only as needed. Assistant unchanged.
- **Learn/libraries:** alignment, embeddings, similarity, false matches, threshold/margin; YuNet + SFace first.
- **Minimum verification/exit:** compact score/margin/quality boundary tests plus one small held-out session evaluation with enrolled and non-enrolled examples.
- **Mistakes/defer:** no family-wide rollout, automated enrollment, tutorial-threshold accuracy claim, vector DB, cloud processing, or account authentication.

### Phase 12B — Opt-in family recognition in shadow mode [LATER; depends on 7, 11C, 12A]

- **Goal/why:** validate multiple household identities, lifecycle and ambiguity before personalization.
- **Architecture:** quality-gated matching → temporal consistency → versioned candidate observation → Rust; no action authority.
- **Files:** household enrollment policy, perception/gallery lifecycle, observations/storage; no calendar authority changes.
- **Learn/libraries:** gallery versioning, revocation, session-scoped tracks, model compatibility; reuse matching stack.
- **Minimum verification/exit:** unknown/ambiguous case; revoked/stale-gallery case; one held-out family session. Keep shadow mode if evidence is inadequate.
- **Mistakes/defer:** no guest identification, door access, private calendar disclosure, automated movement profiling, or spoof-resistance claims.

### Phase 13 — One offline occupancy model [LATER; depends on useful observations]

- **Goal/why:** learn ML methodology from household events, initially without identity.
- **Architecture:** bounded dataset export → offline features → baseline/model comparison.
- **Files:** storage export operation; new experiments and dataset manifest.
- **Learn/libraries:** labels, missingness, temporal splits, calibration; scikit-learn and numerical tools.
- **Minimum verification/exit:** one reproducible held-out temporal evaluation versus one baseline; failure to improve is a valid result.
- **Mistakes/defer:** no adjacent-frame leakage, identity-profile scope creep, live training, or prediction-triggered actuation.

### Phase 14 — One artifact deployed in shadow mode [LATER; depends on 13]

- **Goal/why:** learn model deployment and rollback without giving predictions authority.
- **Architecture:** approved manifest/artifact → inference worker → versioned prediction.
- **Files:** deployment/artifact records, observation contract, perception or one ML worker.
- **Learn/libraries:** preprocessing parity, checksum, runtime compatibility, drift; chosen runtime only.
- **Minimum verification/exit:** one parity sample set, one invalid-version/checksum rejection, and rollback smoke check.
- **Mistakes/defer:** no silent hot replacement, direct model authority, online training, or mixing incompatible embeddings.

### Phase 15A — Push-to-talk input [LATER; depends on 5–6]

- **Goal/why:** add voice without changing reasoning architecture.
- **Architecture:** capture worker → bounded audio → STT adapter → ordinary request.
- **Files:** App/runtime; new audio capture/STT modules.
- **Learn/libraries:** sample formats, buffers, latency; CPAL or OS capture plus Whisper.cpp evaluation.
- **Minimum verification/exit:** one utterance; recording at maximum duration and just beyond stops/rejects predictably; cancellation works.
- **Mistakes/defer:** no always-listening, speaker authentication, unlimited buffering, or automatic cloud transcription.

### Phase 15B — Speak one response and stop [LATER; depends on audio lifecycle]

- **Goal/why:** add controlled output.
- **Architecture:** response → TTS adapter → bounded playback.
- **Files:** audio output and App presentation.
- **Learn/libraries:** playback buffering/interruption; one evaluated Piper or sherpa-onnx voice.
- **Minimum verification/exit:** one response and interruption/queue-limit case; next turn remains usable.
- **Mistakes/defer:** no full duplex, speech agents, simultaneous voices, or private-calendar narration based solely on face recognition.

### Phase 16 — One UI or API [LATER; depends on 6]

- **Goal/why:** expose existing operations without duplicating policy.
- **Architecture:** interface → trusted command → application; render typed state.
- **Files:** App/runtime; one interfaces module.
- **Learn/libraries:** event loop, state snapshots, authentication context; Slint or Axum first, not both.
- **Minimum verification/exit:** responsive interface during model wait; denied command has same outcome as CLI.
- **Mistakes/defer:** no logic in callbacks, public unauthenticated endpoints, or face match treated as logged-in user.

### Phase 17 — One offline improvement proposal [LATER, LAST]

- **Goal/why:** learn controlled evaluation/promotion.
- **Architecture:** candidate prompt/config → isolated evaluation → report → explicit promotion → rollback.
- **Files:** experiments/evaluation/deployment records; runtime need not change.
- **Learn/libraries:** regression comparison, reproducibility, version control; reuse tooling.
- **Minimum verification/exit:** one candidate rejected or approved from fixed evidence; previous version can be restored.
- **Mistakes/defer:** no production-write credentials for assistants, autonomous code deployment, or tuning only against evaluation examples.

## 14. Security, observability, and open decisions

Log run ID, route, stage, tool name, policy outcome, execution outcome, duration, token-count provenance, context decisions and budget use. Do not log full calendar descriptions, camera credentials, face crops/embeddings, raw audio, prompts, or reasoning by default.

Tracing is not a durable audit. Before calendar writes or device actuation, preserve intended action, approval, attempt, and authoritative/uncertain outcome. Separate service identity, household person ID, face candidate, and caller authority.

The current local API key value is a compatibility placeholder, not meaningful authentication. Protect cross-node sensitive traffic and verify deployment controls. Camera-local RTSP and Rust event transport have different security properties.

| Unresolved fact/decision | When to settle | Default until then |
|---|---|---|
| C100 hardware revision/firmware/local account | Before 11B | Model confirmed; verify local stream separately from existing app access |
| Camera location/face quality | Before 12A | Anonymous occupancy only |
| Calendar provider/accounts | Before 8A | One provider adapter; no assumed Google/Outlook/iCloud credentials |
| AI HAT model | Before acceleration | CPU baseline; supported vendor path after identification |
| Deadline/resource budgets | Before production concurrency | Finite measured limits, one interactive run |
| Effective model context window/counting fidelity | Before 4C | Deployed limit, reserved output, labelled estimates and conservative headroom |
| Session history retention and compaction | Before 7B/7C | Bounded memory; no persistence or summarization without need |
| Storage scope, disk, host, retention | Before 11D | One-camera private recording; general file storage/off-site backup separate |
| Remote playback audience and connectivity | Before 11E | VPN plus explicit playback access; home outages remain visible |
| Enrollment and retention preferences | Before 12A/12B | Deliberate opt-in, local minimal data, no raw recording |
| Private calendar disclosure in shared rooms | Before voice calendar output | Explicit authenticated context; do not infer privacy from face presence |
| Camera event reliability | Before automations rely on it | Gaps visible, stale/unknown state |
| Calendar approval and retry semantics | Before 8B | Content-bound approval, no blind retry |
| Slint backend/license | Before 16 | Evaluate actual embedded deployment |

## 15. SKILLS USED

- [grilling](/Users/darrenng/.agents/skills/grilling/SKILL.md): challenged assumptions, prerequisite decisions, premature abstractions, and the identity/authority boundary. Its interview process is adapted to the requested completed plan.
- [find-skills](/Users/darrenng/.agents/skills/find-skills/SKILL.md): guided broader local and external discovery in the original review.
- [codebase-design](/Users/darrenng/.agents/skills/codebase-design/SKILL.md): deletion test for wrappers, narrow interfaces, real substitution before traits.
- [api-and-interface-design](/Users/darrenng/.agents/skills/api-and-interface-design/SKILL.md): tool, calendar, identity-observation, and cross-process contracts.
- [security-and-hardening](/Users/darrenng/.agents/skills/security-and-hardening/SKILL.md): untrusted model data, privacy, permissions, consumption limits; web-specific prescriptions were not adopted indiscriminately.
- [rust-async-patterns](https://github.com/wshobson/agents/blob/main/plugins/systems-programming/skills/rust-async-patterns/SKILL.md): discovered/read, not installed; informed bounded concurrency and supervision, cross-checked with official Tokio documentation.

## NEXT LEARNING SLICE

Add one explicit completion deadline inside [LocalLlm](/Users/darrenng/Desktop/Desktop/personalProjects/jarvis/src/clients/local_llm.rs).

Keep the existing prompt, ownership and output. Learn how a timeout wraps a future and how its error differs from the model request error.

Minimum verification: one normal Pi completion and one deterministic stalled-request timeout. Finish when normal behavior remains intact and the stalled request returns a clear timeout within the chosen budget.

Do not add tools, traits, new modules, retries, cameras, calendar access, facial recognition, or cloud routing in this slice. Darren implements it in a separate guided coding session.
