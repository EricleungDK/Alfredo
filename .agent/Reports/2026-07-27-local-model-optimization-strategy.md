# Alfredo local-model optimization strategy

**Researched:** 2026-07-27

**Question:** Given Alfredo's measured hardware and model baselines, which model selection, quantization, context, prompt, scheduling, caching, concurrency, and Ollama/runtime changes can materially improve time to first token (TTFT), throughput, and goal-to-reviewed-Evidence-Package latency without weakening evidence quality or governance?

**Scope:** planning evidence only. No model, Ollama service, runner, prompt, scheduler, or product configuration was changed.

## Answer

Alfredo should optimize the local-model path as a **single-GPU, quality-gated service** built from explicit [Local Inference Profiles](../../CONTEXT.md#local-inference-profile) and [Local Inference Leases](../../CONTEXT.md#local-inference-lease), not as an unconstrained set of parallel model subprocesses.

The recommended route is:

1. replace the current opaque `ollama run` subprocess boundary with the local Ollama HTTP API so Alfredo can set and record `keep_alive`, `num_ctx`, thinking, sampling, output limits, and a JSON schema, consume streamed chunks, and persist Ollama's load/prompt/evaluation metrics;
2. keep one quality-qualified interactive model resident, grant one Local Inference Lease at a time on the measured 12 GB GPU, and prefer model affinity over controller/worker model swapping;
3. set context per request from measured prompt tokens rather than accepting the sub-24-GB default, while reducing irrelevant repository text and never dropping the authoritative task contract, allowed paths, commands, repair feedback, or evidence requirements;
4. retain the installed Q4_K_M weight quantizations as the initial quality floor, test `q8_0` KV-cache quantization as a reversible memory optimization, and reject lower-precision weights or `q4_0` KV cache unless Alfredo's representative evidence-quality suite proves non-inferiority;
5. choose the normal model by **reviewed outcome latency**, not tokens per second: benchmark Gemma 4 12B, Qwen3 14B, Qwen2.5-Coder 14B, and a smaller controller candidate on the same governed tasks, then reserve the installed 17 GB models for quality-triggered escalation only if fewer failures and repairs offset CPU offload and load cost;
6. upgrade Ollama from the measured 0.30.6 to a pinned, current non-withdrawn release only after replaying that same suite. Later official releases include prompt-cache reuse improvements, a structured-output fix for thinking-disabled models, and Gemma 4 continuation fixes that directly overlap Alfredo's current workload.

Streaming improves true provider TTFT observability and perceived latency, but a partial JSON file plan must remain **non-authoritative**. Alfredo may project progress while tokens arrive; only the complete, bounded, schema-valid plan may pass through the existing path, command, evidence, and review gates.

## Sources and confidence convention

This note separates:

- **Sourced fact** — directly supported by Alfredo's measured report, local read-only inspection, official documentation/source/release notes, or a model creator's model card.
- **Recommendation / inference** — a proposed Alfredo decision derived from those facts. It is not represented as an upstream performance guarantee.

External sources are primary:

- official [Ollama API documentation](https://docs.ollama.com/api/introduction), [FAQ](https://docs.ollama.com/faq), [source](https://github.com/ollama/ollama), and [release notes](https://github.com/ollama/ollama/releases);
- official model cards from [Google DeepMind](https://ai.google.dev/gemma/docs/core/model_card_4), [Qwen](https://huggingface.co/Qwen/Qwen3-14B), and [DeepSeek](https://huggingface.co/deepseek-ai/DeepSeek-R1-Distill-Qwen-14B);
- official [llama.cpp quantization](https://github.com/ggml-org/llama.cpp/blob/master/tools/quantize/README.md), [perplexity](https://github.com/ggml-org/llama.cpp/blob/master/tools/perplexity/README.md), and [server](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) documentation.

## Measured Alfredo baseline

### Sourced facts

The authoritative [architecture and performance baseline](./2026-07-23-alfredo-architecture-performance-baseline.md) measured:

| Boundary | Result |
|---|---:|
| Fresh Python backend to ready Workspace Session / Active Mission | 185.117 ms median |
| Warm persistent snapshot | 9.155 ms median / 11.847 ms p95 |
| Approved Queue intent to durable queued Local Agent session, service only | 2.010 ms median |
| Approval request to public queued acknowledgement | 12.296 ms median |
| Approval-request start to persisted `runner_started_at` | 85.377 ms median |
| Tiny `qwen3:14b` request, model unloaded first | 4.507 s median; 3.910 s median load |
| Tiny `qwen3:14b` request, resident | 2.172 s median; 9.800 s p95 |
| Trivial governed Gemma 4 12B goal to reviewed Evidence Package | 37.842 s |

The machine was Ubuntu 24.04 under WSL2 with an Intel i5-14400F, 15 GiB WSL memory, and an NVIDIA RTX 4070 exposing 12,282 MiB VRAM. Ollama server/client was 0.30.6. The sampled control plane was therefore millisecond-scale while model execution dominated the useful live path.

The qwen micro-measurement used the API with `think: false`, `temperature: 0`, `seed: 42`, `num_predict: 8`, `num_ctx: 2048`, and `keep_alive: "10m"`. The governed Gemma workflow used the production command:

```text
ollama run gemma4:12b --think=false --nowordwrap --format json
```

That worker produced a complete file plan only after the subprocess exited. Alfredo's current implementation does not set worker `num_ctx`, `keep_alive`, seed, temperature, or output-token limit and does not persist Ollama's provider timings. It asks for generic JSON rather than passing a JSON Schema. [Runner command](../../albert_mvp/core.py#L2498-L2503), [worker loop](../../albert_mvp/core.py#L5310-L5479), [worker prompt](../../albert_mvp/core.py#L5820-L5849)

The worker prompt can include a 24,000-character repository context, up to ten source snippets, the task packet, selected skill instructions, and up to 8,000 characters of repair feedback. The controller can include 24,000 characters of recent conversation and accepts a bounded 96,000-character total prompt. These are character bounds, not token-aware context budgets. [Worker limits](../../albert_mvp/core.py#L35-L48), [controller limits](../../albert_mvp/workspace.py#L42-L44), [controller prompt](../../albert_mvp/workspace.py#L3033-L3103)

### Local read-only inspection

`ollama list`, `ollama show`, `ollama ps`, `ollama --version`, and the systemd service properties were inspected without changing state.

| Installed Alfredo model | Local metadata | On-disk size | Initial fit inference |
|---|---|---:|---|
| `gemma4:12b` | 11.9B, 262,144 context, Q4_K_M | 7.6 GB | Best current weight/headroom fit among normal 12B–14B roles |
| `qwen3:14b` | 14.8B, 40,960 context, Q4_K_M | 9.3 GB | Fits weights alone; less KV/context headroom |
| `qwen2.5-coder:14b` | 14.8B, 32,768 context, Q4_K_M | 9.0 GB | Fits weights alone; code-specialized candidate |
| `deepseek-r1:14b` | 14.8B, 131,072 context, Q4_K_M | 9.0 GB | Fits weights alone; reasoning/escalation candidate |
| `gemma4:26b` | 25.8B, 262,144 context, Q4_K_M | 17 GB | Weights exceed available VRAM |
| `qwen3.6:27b` | 27.8B, 262,144 context, Q4_K_M | 17 GB | Weights exceed available VRAM |

No model was resident during inspection. The system service runs plain `ollama serve` and defines none of `OLLAMA_CONTEXT_LENGTH`, `OLLAMA_KEEP_ALIVE`, `OLLAMA_MAX_LOADED_MODELS`, `OLLAMA_NUM_PARALLEL`, `OLLAMA_FLASH_ATTENTION`, or `OLLAMA_KV_CACHE_TYPE`; runtime defaults therefore govern them.

### Fresh bounded planning probes

Three temporary standard-library probes used the local `/api/generate` and `/api/ps` endpoints after confirming `ollama ps` was empty. They changed no model, service, repository, or persistent configuration and unloaded every measured model afterward. Their script SHA-256 values were:

- latency/residency probe: `0336ae01b03d75c33bd4ca5624f34784b7382e50c4ab0fadbd3d949f209bb4c5`;
- prefix probe: `6e35944b3a11f3135fe1b3fc7538702215ad800a20e7fc88850a5898061340f2`;
- context-fit probe: `9f838abcfea7f34fe6616ddd2c4cc2b74dac41bbed680aeed28baf9b0c948ed4`.

The fixed streaming request asked for exactly `READY`, disabled thinking, set temperature zero and seed 42, limited prediction to eight tokens, and allocated 4,096 context tokens. Each model received one model-unloaded request followed by two resident requests:

| Installed Q4_K_M model | Model-unloaded TTFT / total / provider load | Resident TTFT samples | Resident total samples |
|---|---:|---:|---:|
| `qwen3:14b` | 10.804 / 10.904 / 10.600 s | 0.173, 0.163 s | 0.216, 0.206 s |
| `gemma4:12b` | 12.200 / 12.242 / 12.021 s | 0.447, 0.487 s | 0.472, 0.515 s |
| `qwen2.5-coder:14b` | 11.554 / 11.581 / 11.461 s | 0.180, 0.165 s | 0.204, 0.189 s |

These are tiny-output latency samples, not a throughput distribution or a quality comparison. The slower unloaded values than the 2026-07-23 qwen set reinforce the need to report OS/cache/residency state and tails rather than treating one “cold” number as stable. They do show that avoiding a model load was worth roughly ten seconds in this run, while the resident tiny response completed in well under one second for all three installed normal-tier candidates.

A separate resident qwen prefix probe kept the request below the allocated 4K window:

| Prompt shape | Admitted prompt tokens | Prompt evaluation | TTFT |
|---|---:|---:|---:|
| First 3,525-token stable prefix | 3,525 | 1.629 s | 1.798 s |
| Same prefix, changed final request | 3,525 | 0.045 s | 0.222 s |
| Different 3,805-token prefix | 3,805 | 1.659 s | 2.180 s |

This n=1 sequence is direct local evidence that exact-prefix stability can materially reduce prefill on the current runtime. It does not prove a durable cache hit across model swaps, parallel slots, restarts, or changed templates. A deliberately oversized preliminary request reported 4,095 admitted prompt tokens for every variant, demonstrating why a below-ceiling comparison was necessary and why Alfredo must detect context admission explicitly instead of assuming its character bounds fit.

Finally, `/api/ps` reported `size_vram == size` for two proposed starting profiles:

| Profile | Requested context | Reported model size / VRAM | Reported placement inference |
|---|---:|---:|---|
| `qwen3:14b` controller candidate | 8,192 | 10,321,636,883 bytes | Entire reported model allocation in VRAM |
| `gemma4:12b` worker candidate | 16,384 | 8,390,747,094 bytes | Entire reported model allocation in VRAM |

These context values are therefore evidence-backed starting points for the measured machine, not universal defaults. Representative controller and coding tasks must still prove that their mandatory prompt plus output headroom fits and that `ollama ps` continues to show full GPU placement under the promoted Ollama/KV configuration.

On-disk size is not an exact VRAM measurement. Every candidate configuration must be verified under load with `ollama ps`; Ollama documents `100% GPU` versus mixed CPU/GPU in that output. [Official GPU/offload check](https://docs.ollama.com/faq#how-can-i-tell-if-my-model-was-loaded-onto-the-gpu)

## Primary-source findings by optimization lever

### 1. Instrumented API boundary and TTFT

#### Sourced facts

Ollama's `/api/generate` and `/api/chat` endpoints stream newline-delimited JSON by default. The final chunk includes usage metrics. Officially, streaming provides real-time output and lower perceived latency, while non-streaming is simpler for short or structured responses. [Streaming API](https://docs.ollama.com/api/streaming)

Ollama reports, in nanoseconds:

- `total_duration`;
- `load_duration`;
- `prompt_eval_count` and `prompt_eval_duration`;
- `eval_count` and `eval_duration`.

[Official usage metrics](https://docs.ollama.com/api/usage)

The API also exposes request-level `think`, `keep_alive`, `format`, and runtime `options`; `format` accepts either `"json"` or a JSON Schema. [Generate API](https://docs.ollama.com/api/generate), [Chat API](https://docs.ollama.com/api/chat)

#### Recommendation / inference

Make the local Ollama adapter an HTTP client owned by Alfredo rather than a stringly CLI command:

- record monotonic request start, first streamed thinking/content chunk, final chunk, plan validation, first governed command, Evidence Package ready, and review-visible timestamps;
- persist Ollama's own load, prompt-evaluation, and token-evaluation counts/durations beside the model digest, quantization, context, thinking, sampling, residency, and processor split;
- accumulate bounded stream bytes and do not parse, write, or execute the plan until `done: true`;
- surface a non-authoritative `loading`, `prefill`, or `generating` operation state from provider events, but preserve the current authoritative session/evidence lifecycle;
- retain a non-streaming mode in the benchmark because constrained structured output can be simpler and possibly more reliable. Compare it against bounded streaming rather than assuming streaming changes end-to-end completion time.

This change is the prerequisite for defensible TTFT and throughput claims. The current CLI path can measure only subprocess completion.

### 2. Residency and model affinity

#### Sourced facts

Ollama keeps models loaded for five minutes by default. An empty API request preloads a model. `keep_alive` can be a duration, `-1` for indefinite residency, or `0` for immediate unload, and a request-level value overrides the server default. [Official preload and keep-alive behavior](https://docs.ollama.com/faq#how-can-i-preload-a-model-into-ollama-to-get-faster-response-times)

Alfredo's isolated qwen measurement attributed a 3.910-second median to model load and reduced total median from 4.507 seconds unloaded to 2.172 seconds resident, although the resident p95 remained high.

Ollama queues a new model when insufficient memory is available and unloads idle models to make room. Concurrent GPU model loads require the models to fit in available VRAM. [Official scheduling behavior](https://docs.ollama.com/faq#how-does-ollama-handle-concurrent-requests)

#### Recommendation / inference

- Keep the selected interactive model resident for an explicit bounded lease such as 30–60 minutes of workstation activity, with unload on workspace close, model change, memory pressure, or an explicit user action.
- Preload only after the user selects a model or admits local-model work. Do not restore launcher-blocking prewarm; workstation launch must remain usable without Ollama.
- Prefer one model for adjacent controller and normal-worker turns when it passes both quality gates. A 3–4 second swap penalty can erase the advantage of a nominally faster role-specific model.
- If role specialization is materially better, test a small controller (for example, an official 4B-class Qwen or Gemma variant) that may co-reside with Gemma 4 12B. Treat co-residency as unproven until `ollama ps` shows both fully on GPU at the chosen contexts.
- Do not pin multiple 12B–14B models indefinitely on this GPU. The inspected weight sizes alone total more than VRAM before KV caches.

### 3. Context sizing and context quality

#### Sourced facts

Current Ollama documentation defaults to 4K context below 24 GiB VRAM, 32K at 24–48 GiB, and 256K at 48 GiB or more. It also says coding/agent tools often need at least 64K, while warning that larger context consumes more memory and should avoid CPU offload. [Official context-length guidance](https://docs.ollama.com/context-length)

The API can set `options.num_ctx` per request. The Modelfile can set `num_ctx`, and `OLLAMA_CONTEXT_LENGTH` changes the server default. [FAQ context setting](https://docs.ollama.com/faq#how-can-i-specify-the-context-window-size), [Modelfile reference](https://docs.ollama.com/modelfile)

The installed models advertise longer architectural windows than the likely sub-24-GB runtime default. Qwen's official Qwen3-14B card states a 32,768-token native window and 131,072 with YaRN. Qwen2.5-Coder-14B advertises 131,072, while the installed Ollama tag reports 32,768. Gemma 4 12B advertises up to 256K. [Qwen3-14B model card](https://huggingface.co/Qwen/Qwen3-14B), [Qwen2.5-Coder model card](https://huggingface.co/Qwen/Qwen2.5-Coder-14B-Instruct), [Gemma 4 model card](https://ai.google.dev/gemma/docs/core/model_card_4)

#### Recommendation / inference

Do not choose either “smallest possible” or “model maximum” globally. Choose the smallest context that contains the **tokenized, quality-critical prompt plus generation headroom**:

1. count tokens after the real model template is applied;
2. fail closed before inference if the contract plus mandatory instructions exceed the admitted window;
3. reserve output headroom explicitly;
4. begin the measured-machine bake-off with the locally full-GPU 8K qwen controller and 16K Gemma worker profiles, then compare the largest fully-GPU practical context for normal work;
5. admit 32K/64K only for tasks that demonstrate a need and remain fully GPU-resident or whose measured outcome gain justifies offload.

Prompt compaction must preserve, verbatim or structurally:

- task goal and acceptance criteria;
- allowed paths and command policy;
- `AGENTS.md` / governed instructions;
- exact repair feedback and prior accepted evidence needed for repair;
- output schema and evidence requirements.

Compact lower-value material first: redundant prose, unrelated file-tree entries, duplicated task fields, stale conversation, and source snippets with no lexical/semantic relationship to the goal. Cache repository indexes by content digest and make retrieval decisions inspectable in the task packet. A token budget should replace the current character-only admission rule.

This is both a latency and quality correction: an implicit 4K window risks context shifting or omission, while blindly allocating 64K on a 12 GB card risks KV pressure and CPU offload.

### 4. Prompt design, structured output, and output volume

#### Sourced facts

Ollama structured outputs can enforce a JSON Schema. The official guidance recommends also grounding the prompt with the schema and lowering temperature, for example to zero, for deterministic completions. [Official structured-output guidance](https://docs.ollama.com/capabilities/structured-outputs)

Qwen3 officially supports thinking and non-thinking modes. Its creators describe non-thinking as the efficient mode and recommend different sampling settings for thinking and non-thinking; they warn against greedy decoding in thinking mode. [Qwen3-14B model card](https://huggingface.co/Qwen/Qwen3-14B#switching-between-thinking-and-non-thinking-mode)

Gemma 4 supports configurable thinking. Its model card says previous thinking should normally not be retained in multi-turn history except for tool-call turns. [Gemma 4 model card](https://ai.google.dev/gemma/docs/core/model_card_4)

Alfredo currently requests a plan containing complete replacement contents for every changed file. Generation work therefore scales with all returned file bytes, even when the logical edit is small.

#### Recommendation / inference

- Pass the exact file-plan schema to Ollama's `format` field and still validate it independently in Alfredo. Constrained decoding reduces malformed-plan risk; it does not replace path, byte, command, or evidence validation.
- Use non-thinking mode for controller classification and routine bounded edits. Permit thinking only for an explicit complex-task/escalation tier and include its extra generated tokens in outcome latency.
- Tune sampling per model and role. Do not apply one global `temperature: 0` rule to thinking models contrary to their creator guidance. Use fixed seeds only for reproducible benchmark cases; production quality should be evaluated across repeated samples.
- Add a hard `num_predict`/output-token budget derived from allowed file scope and plan schema. Report `length` termination as a typed model failure, never as valid evidence.
- Prototype a smaller edit representation—validated unified diffs or typed edit hunks—so a one-line change does not regenerate a whole file. Deterministically apply it inside the worktree, reject conflicts/escapes, then produce the same real bounded `review.diff` and Evidence Package from disk. Retain full-file replacement as a fallback until representative tasks show the patch form is at least as reliable.
- Longer term, compare the one-shot plan with a bounded read/edit/test tool loop. Every tool remains Orchestrator-mediated; the model receives no direct shell or filesystem authority.

### 5. Weight and KV-cache quantization

#### Sourced facts

All inspected Alfredo models use Q4_K_M weights. llama.cpp documents Q4_K_M as a supported mixed quantization and warns that requantizing already quantized weights can severely reduce quality. Its perplexity tool is explicitly used to assess quantization loss against FP16, while warning that perplexity is not directly comparable across different models/tokenizers and does not replace task evaluation. [llama.cpp quantization](https://github.com/ggml-org/llama.cpp/blob/master/tools/quantize/README.md), [llama.cpp perplexity](https://github.com/ggml-org/llama.cpp/blob/master/tools/perplexity/README.md)

Ollama uses an f16 KV cache by default. With Flash Attention, `q8_0` uses about half the KV memory with a very small precision loss that is usually unnoticeable; `q4_0` uses about one quarter with a small-to-medium loss that can be more noticeable at longer contexts. Ollama specifically notes that Qwen2 models can be more sensitive and says impact depends on model and task. [Official KV-cache guidance](https://github.com/ollama/ollama/blob/main/docs/faq.mdx#how-can-i-set-the-quantization-type-for-the-kv-cache)

#### Recommendation / inference

- Keep Q4_K_M weights as the initial deployment floor. It is already small enough for the 12B–14B candidates to fit their weight files within VRAM, and no Alfredo evidence shows that a lower weight precision preserves coding/evidence quality.
- Do not requantize the installed Q4 artifacts. If a different quantization is tested, pull or produce it from the original higher-precision source with immutable identity.
- A/B `OLLAMA_KV_CACHE_TYPE=q8_0` first. It is the best documented reversible route to more context headroom; require identical governance tests and non-inferior accepted-task/repair rates.
- Test `q4_0` KV only as an explicit constrained-hardware experiment. It should not be the normal default for Qwen roles without stronger task evidence.
- Compare a smaller 7B–8B model at Q5/Q6 against a 12B–14B Q4 model rather than assuming bit width or parameter count alone determines quality or latency.

### 6. Flash Attention and GPU residency

#### Sourced facts

Ollama uses Flash Attention automatically when the backend and devices support it; it can be forced with `OLLAMA_FLASH_ATTENTION=1`. Flash Attention reduces memory growth with context. Quantized KV cache requires it. [Official Flash Attention guidance](https://github.com/ollama/ollama/blob/main/docs/faq.mdx#how-can-i-enable-flash-attention)

Ollama recommends checking `ollama ps`: full single-GPU placement usually performs best, while mixed CPU/GPU indicates offload. [Official GPU placement behavior](https://docs.ollama.com/faq#how-does-ollama-load-models-on-multiple-gpus)

#### Recommendation / inference

- First confirm from server logs and `ollama ps` whether automatic Flash Attention is active for each selected model/backend. Do not assume setting the environment variable proves the kernel is used.
- Benchmark automatic versus forced-on only if logs confirm both are valid. Persist the actual runtime result, not just requested configuration.
- Make `100% GPU` at the admitted context the normal-tier requirement. The 17 GB Q4 models exceed the 12.3 GB VRAM before KV allocation and should be treated as CPU-offloaded escalation candidates, not latency defaults.

### 7. Scheduling, concurrency, and queueing

#### Sourced facts

Ollama exposes:

- `OLLAMA_MAX_LOADED_MODELS`;
- `OLLAMA_NUM_PARALLEL`;
- `OLLAMA_MAX_QUEUE`.

Parallel requests increase memory in proportion to parallel count times context. When a new model cannot fit, requests queue while idle models are unloaded. [Official concurrent-request behavior](https://docs.ollama.com/faq#how-does-ollama-handle-concurrent-requests)

Alfredo's baseline did not characterize concurrent controller plus worker inference, two Local Agents, GPU eviction, or UI polling during inference.

#### Recommendation / inference

For this single 12 GB GPU, begin with one Local Inference Lease at a time and:

```text
OLLAMA_MAX_LOADED_MODELS=1
OLLAMA_NUM_PARALLEL=1
```

and let Alfredo own a bounded, visible inference queue. This avoids unmeasured context multiplication, eviction thrash, and competing long generations. It is intentionally a latency-first baseline, not a universal throughput optimum; the lease orders capacity but never authorizes work or changes accepted Mission state.

Scheduling policy:

- interactive controller turn: highest dispatch priority, bounded output;
- already-generating worker: do not preempt if preemption would discard work; show the controller as queued;
- normal Local Agents: one GPU inference at a time;
- CPU-only tests/builds: may run concurrently under existing resource/governance limits when they do not contend materially with model inference;
- 17 GB escalation model: drain normal GPU work, load explicitly, complete the bounded escalation, then restore the normal resident model;
- co-resident or same-model parallelism: graduate only after a two-request benchmark proves better aggregate accepted-outcome throughput without p95, OOM, offload, or quality regression.

Set `OLLAMA_MAX_QUEUE` to an Alfredo-sized bound rather than the server's broad default, but keep the authoritative queue in Alfredo so cancellation, priority, Mission ownership, and audit semantics remain visible.

### 8. Prompt/KV caching

#### Sourced facts

Ollama 0.30.8 release notes report improved prompt caching by decoupling it from context shift for better KV-cache reuse. [Official v0.30.8 release](https://github.com/ollama/ollama/releases/tag/v0.30.8)

The underlying llama.cpp server documents prompt-prefix reuse: when caching is enabled, it compares the prompt to prior work and evaluates only an unseen suffix; its timing response reports cached versus newly evaluated prompt tokens. [llama.cpp server prompt-cache documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)

The measured machine still runs Ollama 0.30.6.

The fresh local prefix probe is consistent with reuse on that runtime: changing only the final request reduced 3,525-token prompt evaluation from 1.629 seconds to 45 milliseconds, while replacing the prefix returned prompt evaluation to 1.659 seconds. Because the sequence was small and single-slot, the upgrade and repeated bake-off remain necessary.

#### Recommendation / inference

- Upgrade before relying on prompt-prefix caching.
- Put the stable contract/schema/instructions first and volatile user/task/repository material later so repeated requests share the longest exact prefix.
- Keep one model and one normal inference slot while establishing cache behavior; model swaps and parallel slots complicate reuse.
- Use `prompt_eval_count` and `prompt_eval_duration` to prove reuse. Do not claim caching from prompt shape alone.
- Cache deterministic repository selection/index data by content digest, but do not cache an old model plan or Evidence Package as if it applied to changed source state.
- Never allow a cache hit to bypass fresh expected-revision, workspace snapshot, allowed-path, command, test, evidence, or review checks.

### 9. Ollama/runtime version

#### Sourced facts

The measured runtime is 0.30.6. Official releases after it include:

- 0.30.8: improved prompt-cache reuse;
- 0.31.2: fixed structured output for thinking models when thinking is disabled and updated the llama.cpp engine;
- 0.32.1: improved Gemma 4 tool calling and multi-turn/tool-response continuation reliability;
- 0.32.2 was explicitly withdrawn in favor of 0.32.3 or newer;
- the official release list shows newer non-withdrawn releases through 0.32.5 on the research date.

[Official Ollama releases](https://github.com/ollama/ollama/releases)

Gemma 4's model overview says its models contain a draft model for multi-token-prediction speculative decoding. Ollama's published Gemma 4 MTP speedup release is specifically for its MLX runner on Macs; that is not evidence of support or speedup on Alfredo's Linux/NVIDIA CUDA path. [Gemma 4 overview](https://ai.google.dev/gemma/docs/core), [Ollama v0.23.1 MTP release](https://github.com/ollama/ollama/releases/tag/v0.23.1)

#### Recommendation / inference

- Use 0.31.2 as the minimum upgrade candidate because it contains the directly relevant structured-output fix; compare it with a current non-withdrawn release, then pin the version that passes Alfredo's suite rather than tracking latest blindly.
- Replay model-load, non-thinking structured output, context, Flash Attention/KV, cancellation, malformed-output, and Evidence Package tests before promotion.
- Preserve the prior Ollama binary/config as a rollback path until the representative suite and a real workstation soak pass.
- Do not plan on speculative decoding for this RTX 4070 until official CUDA support and a matching model tag are demonstrated. The documented MLX result does not transfer to this machine.

## Model-selection strategy

### Sourced facts

The model creators make different capability claims:

- Gemma 4 12B is intended for laptops/desktops and supports coding, reasoning, function calling, configurable thinking, and up to 256K context. Google's producer benchmark reports 72.0 on LiveCodeBench v6 for the instruction-tuned 12B, but that result is not a measurement of Alfredo's Q4_K_M tag or workflow. [Gemma 4 model card](https://ai.google.dev/gemma/docs/core/model_card_4)
- Qwen3-14B supports thinking/non-thinking, code generation, tool use, and agent tasks. [Qwen3-14B model card](https://huggingface.co/Qwen/Qwen3-14B)
- Qwen2.5-Coder-14B is code-specific and its creator reports training for code generation, reasoning, fixing, and code-agent applications. [Qwen2.5-Coder-14B model card](https://huggingface.co/Qwen/Qwen2.5-Coder-14B-Instruct)
- Qwen3.6-27B's creator emphasizes agentic coding and repository-level reasoning, but the installed Q4 tag is 17 GB and therefore exceeds Alfredo's VRAM before KV cache. [Qwen3.6-27B model card](https://huggingface.co/Qwen/Qwen3.6-27B)
- DeepSeek-R1-Distill-Qwen-14B is a reasoning-distilled Qwen2.5 model, not a purpose-built normal-latency file-plan worker. [DeepSeek model card](https://huggingface.co/deepseek-ai/DeepSeek-R1-Distill-Qwen-14B)

Producer benchmarks use different prompts, precision, runtimes, and evaluation protocols. They cannot select Alfredo's winner.

### Recommendation / inference

Run one governed bake-off with immutable task fixtures:

| Candidate | Role to test | Reason |
|---|---|---|
| Gemma 4 12B Q4_K_M | unified controller + normal worker; worker-only | Smallest installed normal worker, strongest VRAM headroom, already produced valid evidence |
| Qwen3 14B Q4_K_M | controller; unified controller + worker | Current controller, non-thinking support, measured residency baseline |
| Qwen2.5-Coder 14B Q4_K_M | normal code worker | Code-specialized installed comparator |
| New 4B-class official instruct/coder model | controller only | Potential low-TTFT model that may co-reside |
| Gemma 4 26B / Qwen3.6 27B Q4_K_M | escalation only | Test whether fewer failures/repairs offset offload and load latency |
| DeepSeek-R1 14B Q4_K_M | hard reasoning/diagnosis escalation | Reasoning candidate; expected token overhead is inappropriate for routine work |

Select separately for:

- discussion and route classification;
- small bounded edits;
- multi-file edits;
- test-failure repair;
- diagnosis/reasoning escalation.

A single unified model is preferred only if it meets every applicable quality gate. A role-specific model earns its swap/load cost only when its lower failure/repair rate improves total reviewed-outcome latency.

## Required benchmark and non-regression gate

No configuration should be called faster or promoted from research based on one tiny completion, one trivial edit, model-card benchmark, or tokens per second alone.

### Workload

Use at least:

1. controller discussion that must not create work;
2. explicit coding request that must produce the exact route contract;
3. one-file exact-content edit;
4. small existing-file modification;
5. multi-file change with focused tests;
6. first-test failure followed by governed repair;
7. malformed/oversized model output;
8. attempted write outside `allowed_paths`;
9. attempted non-approved command;
10. cancellation while loading, prefilling, generating, and running a command;
11. representative long-context task where the required source lies near the context boundary;
12. model swap and two queued Local Agents.

Repeat enough times to report median and tail behavior. Separate cold server, model-unloaded, resident/cache-cold, and resident/prefix-reused samples.

### Metrics

Record:

- monotonic request-to-load-start, load end, first chunk, final chunk;
- `load_duration`, prompt tokens and prompt tokens/second;
- output tokens and output tokens/second;
- context allocated, actual prompt tokens, truncation/context-shift events;
- GPU/CPU processor split and peak VRAM/RAM;
- queue wait and model swaps;
- schema-valid-plan rate;
- command-policy/path-policy rejection accuracy;
- task acceptance and independent test pass rate;
- Evidence Package validity/completeness;
- first-pass success, repair count, and escalation count;
- goal-to-evidence-ready, goal-to-review-visible, and goal-to-accepted-review latency;
- human review dwell time separately, never folded into model latency.

### Promotion rule

Promote a model/runtime/configuration only when:

- governance violations remain zero;
- every accepted Evidence Package still comes from the actual bounded worktree diff, commands, tests, risks, and context proposal;
- malformed, truncated, cancelled, or provider-failed output never appears as success;
- route/plan/evidence validity and representative task acceptance are non-inferior to the current Q4_K_M baseline;
- latency improvement survives repeated resident and cold samples and does not hide a worse p95 or extra repairs;
- the configuration, exact model identity, runtime version, measurements, and rollback are recorded.

## Prioritized decision

| Priority | Change to prototype/measure | Expected mechanism | Quality/governance condition |
|---|---|---|---|
| P0 | HTTP streaming/API adapter plus monotonic phase metrics | Exposes actual TTFT, load, prefill, decode, and cache behavior; enables per-request controls | Partial streams never become plans; full schema and existing gates remain |
| P0 | Explicit token-aware context and output headroom | Prevents hidden 4K truncation and avoids needless KV/offload | Mandatory contract/instructions/evidence content cannot be compacted away |
| P0 | Representative model/config bake-off | Selects for reviewed outcome rather than synthetic tok/s | Same governed fixtures and independent review |
| P1 | One resident normal model, model-affinity scheduling, one GPU request | Removes measured load cost and prevents eviction/contention | Visible bounded queue, cancellation, and audit stay authoritative in Alfredo |
| P1 | Upgrade and pin current non-withdrawn Ollama | Gains upstream cache/structured/Gemma fixes | Full regression suite and binary rollback |
| P1 | JSON Schema constrained plan, explicit model-specific thinking/sampling | Fewer malformed plans and unnecessary reasoning tokens | Alfredo still independently validates every field/effect |
| P1 | Flash Attention verification and q8_0 KV A/B | More context headroom, possibly full-GPU placement | Logs prove activation; task quality is non-inferior |
| P2 | Prompt-prefix stabilization and digest-keyed repository retrieval cache | Lower repeated prefill and fewer irrelevant tokens | Cache never bypasses fresh source/authority checks |
| P2 | Typed edit hunks/diff prototype | Fewer generated tokens for small edits | Deterministic safe apply and same real Evidence Package |
| P2 | Small co-resident controller candidate | Lower interactive TTFT without swapping worker | Both models fully fit; route quality and p95 pass |
| P3 | 17 GB escalation models | Potentially fewer failures on hard work | Use only when total reviewed latency wins despite offload |
| Defer | Weight quantization below Q4_K_M, q4_0 KV default | Saves memory | Insufficient Alfredo quality evidence |
| Defer | Parallel GPU inference on this card | Potential aggregate throughput | Concurrency baseline absent; memory scales with parallel contexts |
| Defer | Speculative decoding | Potential decode speedup | Official cited support is MLX/Mac, not Alfredo's CUDA path |

## Resolution gist

Alfredo should retain Q4_K_M as its quality floor, move local inference to an instrumented structured Ollama API, use explicit token-aware context, keep one quality-qualified model resident, and serialize GPU-heavy work on the 12 GB RTX 4070. Model choice must come from a governed, repeated bake-off measuring goal-to-reviewed Evidence Package—not raw token speed—with Gemma 4 12B, Qwen3 14B, and Qwen2.5-Coder 14B as normal-tier candidates and the 17 GB models as escalation-only. Upgrade Ollama under regression/rollback control, then test prompt-prefix reuse, verified Flash Attention, and q8_0 KV cache; do not weaken plan, path, command, Evidence Package, or human-review gates for latency.
