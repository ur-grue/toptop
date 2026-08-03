//! Local-AI workload detection.
//!
//! Recognizes the processes AI engineers actually run locally — both inference
//! *serving* runtimes and *training* launchers — so the AI view can call them
//! out by name, group them, and join them to CPU/RAM/VRAM. Matching is
//! deliberately specific (whole tokens, not bare "llama") to avoid false
//! positives, and checks both the process name and the full command line so
//! Python-launched workloads (e.g. `python -m vllm…`) are caught too.

/// Whether a detected workload serves inference or runs training.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiKind {
    Serving,
    Training,
}

/// A recognized local-AI runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub label: &'static str,
    pub kind: AiKind,
}

use AiKind::{Serving, Training};

/// Known runtimes as `(needle, label, kind)`. The needle is matched
/// case-insensitively at token boundaries against the process name and cmdline.
const RUNTIMES: &[(&str, &str, AiKind)] = &[
    // ── Serving ──────────────────────────────────────────────────────────────
    ("ollama", "Ollama", Serving),
    ("llama-server", "llama.cpp", Serving),
    ("llama-cli", "llama.cpp", Serving),
    ("llama-bench", "llama.cpp", Serving),
    ("llamafile", "llamafile", Serving),
    ("vllm", "vLLM", Serving),
    ("sglang", "SGLang", Serving),
    ("text-generation-server", "TGI", Serving),
    ("tgi", "TGI", Serving),
    ("koboldcpp", "KoboldCpp", Serving),
    ("exllamav2", "ExLlamaV2", Serving),
    ("exllama", "ExLlama", Serving),
    ("lmstudio", "LM Studio", Serving),
    ("lm-studio", "LM Studio", Serving),
    ("localai", "LocalAI", Serving),
    ("gpt4all", "GPT4All", Serving),
    ("mlx_lm", "MLX", Serving),
    ("mlx-lm", "MLX", Serving),
    ("tabbyapi", "TabbyAPI", Serving),
    ("comfyui", "ComfyUI", Serving),
    ("text-generation-webui", "TGW", Serving),
    ("mlc_llm", "MLC LLM", Serving),
    // ── Training / fine-tuning ───────────────────────────────────────────────
    ("torchrun", "PyTorch", Training),
    ("deepspeed", "DeepSpeed", Training),
    ("axolotl", "Axolotl", Training),
    ("unsloth", "Unsloth", Training),
    ("llamafactory", "LLaMA-Factory", Training),
    ("llama-factory", "LLaMA-Factory", Training),
    ("torchtune", "torchtune", Training),
    ("megatron", "Megatron", Training),
    ("litgpt", "LitGPT", Training),
    ("nanogpt", "nanoGPT", Training),
];

/// Return the runtime if `name`/`cmd` look like a local-AI workload.
pub fn detect_runtime(name: &str, cmd: &str) -> Option<Runtime> {
    let name = name.to_ascii_lowercase();
    let cmd = cmd.to_ascii_lowercase();
    for (needle, label, kind) in RUNTIMES {
        if token_match(&name, needle) || token_match(&cmd, needle) {
            return Some(Runtime { label, kind: *kind });
        }
    }
    None
}

/// Backwards-compatible helper returning just the runtime label.
pub fn inference_runtime(name: &str, cmd: &str) -> Option<&'static str> {
    detect_runtime(name, cmd).map(|r| r.label)
}

/// Match `needle` in `hay` only at a token boundary (non-alphanumeric on each
/// side), so `vllm` matches `python -m vllm.entrypoints` but not `svllmx`.
fn token_match(hay: &str, needle: &str) -> bool {
    let bytes = hay.as_bytes();
    let nlen = needle.len();
    let mut start = 0;
    while let Some(pos) = hay[start..].find(needle) {
        let i = start + pos;
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after = i + nlen;
        let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = i + nlen;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_serving_by_name() {
        assert_eq!(inference_runtime("ollama", "ollama serve"), Some("Ollama"));
        let r = detect_runtime("llama-server", "/opt/llama-server -m m.gguf").unwrap();
        assert_eq!(r.label, "llama.cpp");
        assert_eq!(r.kind, AiKind::Serving);
    }

    #[test]
    fn detects_python_launched() {
        assert_eq!(
            inference_runtime("python3", "python3 -m vllm.entrypoints.openai.api_server"),
            Some("vLLM")
        );
        assert_eq!(
            inference_runtime("python", "python -m sglang.launch_server"),
            Some("SGLang")
        );
    }

    #[test]
    fn detects_training() {
        let r = detect_runtime("python", "torchrun --nproc_per_node 8 train.py").unwrap();
        assert_eq!(r.kind, AiKind::Training);
        assert_eq!(r.label, "PyTorch");
        assert_eq!(
            detect_runtime("accelerate", "axolotl train cfg.yml").map(|r| r.kind),
            Some(AiKind::Training)
        );
    }

    #[test]
    fn avoids_false_positives() {
        assert_eq!(inference_runtime("bash", "bash -c 'echo hi'"), None);
        assert_eq!(inference_runtime("svllmx", "/usr/bin/svllmx"), None);
    }

    #[test]
    fn detects_mlc_llm() {
        let r = detect_runtime("mlc_llm", "mlc_llm serve HF://mlc-ai/Llama-3-8B-Instruct").unwrap();
        assert_eq!(r.label, "MLC LLM");
        assert_eq!(r.kind, AiKind::Serving);
        assert_eq!(
            inference_runtime("python3", "python3 -m mlc_llm.serve --model ./dist/model"),
            Some("MLC LLM")
        );
    }
}
