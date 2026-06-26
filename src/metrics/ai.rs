//! Local-inference runtime detection.
//!
//! Recognizes the processes AI engineers actually run locally so the AI view
//! can call them out by name and join them to their CPU/RAM/VRAM usage. Matching
//! is deliberately specific (whole tokens, not bare "llama") to avoid false
//! positives, and checks both the process name and its full command line so
//! Python-launched servers (e.g. `python -m vllm...`) are caught too.

/// Known local-inference runtimes, as `(needle, label)` pairs. The needle is
/// matched case-insensitively against the process name and command line.
const RUNTIMES: &[(&str, &str)] = &[
    ("ollama", "Ollama"),
    ("llama-server", "llama.cpp"),
    ("llama-cli", "llama.cpp"),
    ("llama-bench", "llama.cpp"),
    ("llamafile", "llamafile"),
    ("vllm", "vLLM"),
    ("sglang", "SGLang"),
    ("text-generation-server", "TGI"),
    ("tgi", "TGI"),
    ("koboldcpp", "KoboldCpp"),
    ("exllamav2", "ExLlamaV2"),
    ("exllama", "ExLlama"),
    ("lmstudio", "LM Studio"),
    ("lm-studio", "LM Studio"),
    ("localai", "LocalAI"),
    ("gpt4all", "GPT4All"),
    ("mlx_lm", "MLX"),
    ("mlx-lm", "MLX"),
    ("tabbyapi", "TabbyAPI"),
    ("comfyui", "ComfyUI"),
    ("text-generation-webui", "TGW"),
];

/// Return the runtime label if `name`/`cmd` look like a local-inference process.
pub fn inference_runtime(name: &str, cmd: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    let cmd = cmd.to_ascii_lowercase();
    for (needle, label) in RUNTIMES {
        if token_match(&name, needle) || token_match(&cmd, needle) {
            return Some(label);
        }
    }
    None
}

/// Match `needle` in `cmd` only at a token boundary (non-alphanumeric on each
/// side), so `vllm` matches `python -m vllm.entrypoints` but not `svllmx`.
fn token_match(cmd: &str, needle: &str) -> bool {
    let bytes = cmd.as_bytes();
    let nlen = needle.len();
    let mut start = 0;
    while let Some(pos) = cmd[start..].find(needle) {
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
    fn detects_by_name() {
        assert_eq!(inference_runtime("ollama", "ollama serve"), Some("Ollama"));
        assert_eq!(
            inference_runtime("llama-server", "/opt/llama-server -m m.gguf"),
            Some("llama.cpp")
        );
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
    fn avoids_false_positives() {
        assert_eq!(inference_runtime("bash", "bash -c 'echo hi'"), None);
        // 'vllm' embedded in a larger token must not match.
        assert_eq!(inference_runtime("svllmx", "/usr/bin/svllmx"), None);
    }
}
