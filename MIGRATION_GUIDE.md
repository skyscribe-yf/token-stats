# Pi → Kimi-Code Model Configuration Migration Guide

## Overview

All 6 providers in your pi `models.json` can be migrated to kimi-code's `config.toml`. All use standard protocols (OpenAI-compatible, Anthropic Messages, or OpenAI Responses).

## Provider-by-Provider Analysis

| # | Provider | Pi API Type | Kimi-Code Type | Status |
|---|----------|-------------|----------------|--------|
| 1 | `deepseek` | openai-completions | `openai` | ✅ |
| 2 | `opencode-go` | openai-completions | `openai` | ✅ |
| 3 | `xiaomi-mimo-tp` | openai-completions | `openai` | ✅ |
| 4 | `freemodel` | openai-completions | `openai` | ✅ |
| 5 | `xunfei` | anthropic-messages | `anthropic` | ✅ |
| 6 | `ainaiba` | openai-responses | `openai_responses` | ✅ |

---

## Complete config.toml Addition

Add this to your `~/.kimi-code/config.toml`:

```toml
# ============================================================
# PROVIDERS
# ============================================================

# 1. DeepSeek (direct API)
[providers.deepseek]
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key = "sk-53083d28fe4b42328eb3daa64f46cc22"

# 2. OpenCode Go
[providers.opencode-go]
type = "openai"
base_url = "https://opencode.ai/zen/go/v1"
api_key = "YOUR_OPENCODE_API_KEY_HERE"

# 3. XiaoMi MiMo Token Plan
[providers.xiaomi-mimo-tp]
type = "openai"
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
api_key = "YOUR_XIAOMI_TP_API_KEY_HERE"

# 4. FreeModel
[providers.freemodel]
type = "openai"
base_url = "https://api.freemodel.dev/v1"
api_key = "YOUR_FREE_MODEL_API_KEY_HERE"

# 5. Xunfei (讯飞星火) - Anthropic Messages protocol
[providers.xunfei]
type = "anthropic"
base_url = "https://maas-coding-api.cn-huabei-1.xf-yun.com/anthropic"
api_key = "YOUR_XUNFEI_API_KEY_HERE"

# 6. Ainaiba (OpenAI GPT Private)
[providers.ainaiba]
type = "openai_responses"
base_url = "https://api-xai.ainaibahub.com/v1"
api_key = "YOUR_YAI_API_KEY_HERE"

# ============================================================
# MODEL ALIASES
# ============================================================

# --- DeepSeek (direct) ---
[models."deepseek/deepseek-v4-flash"]
provider = "deepseek"
model = "deepseek-v4-flash"
max_context_size = 131072
display_name = "DeepSeek V4 Flash"

[models."deepseek/deepseek-v4-pro"]
provider = "deepseek"
model = "deepseek-v4-pro"
max_context_size = 131072
display_name = "DeepSeek V4 Pro"

# --- OpenCode Go ---
[models."opencode-go/deepseek-v4-flash"]
provider = "opencode-go"
model = "deepseek-v4-flash"
max_context_size = 1000000
display_name = "DeepSeek V4 Flash (OC)"

[models."opencode-go/deepseek-v4-pro"]
provider = "opencode-go"
model = "deepseek-v4-pro"
max_context_size = 1000000
display_name = "DeepSeek V4 Pro (OC)"

[models."opencode-go/kimi-k2.5"]
provider = "opencode-go"
model = "kimi-k2.5"
max_context_size = 262144
capabilities = ["thinking", "image_in"]
display_name = "Kimi K2.5 (OC)"

[models."opencode-go/kimi-k2.6"]
provider = "opencode-go"
model = "kimi-k2.6"
max_context_size = 262144
capabilities = ["thinking", "image_in"]
display_name = "Kimi K2.6 (OC)"

[models."opencode-go/mimo-v2.5"]
provider = "opencode-go"
model = "mimo-v2.5"
max_context_size = 131072
capabilities = ["thinking", "image_in"]
display_name = "MiMo V2.5 (OC)"

[models."opencode-go/glm-5"]
provider = "opencode-go"
model = "glm-5"
max_context_size = 202752
capabilities = ["thinking"]
display_name = "GLM-5 (OC)"

[models."opencode-go/glm-5.1"]
provider = "opencode-go"
model = "glm-5.1"
max_context_size = 202752
capabilities = ["thinking"]
display_name = "GLM-5.1 (OC)"

# --- XiaoMi MiMo Token Plan ---
[models."xiaomi-mimo-tp/mimo-v2.5-pro"]
provider = "xiaomi-mimo-tp"
model = "mimo-v2.5-pro"
max_context_size = 1048576
capabilities = ["thinking", "image_in"]
display_name = "MiMo V2.5 Pro (TP)"

[models."xiaomi-mimo-tp/mimo-v2.5"]
provider = "xiaomi-mimo-tp"
model = "mimo-v2.5"
max_context_size = 131072
capabilities = ["thinking", "image_in"]
display_name = "MiMo V2.5 (TP)"

[models."xiaomi-mimo-tp/mimo-v2-pro"]
provider = "xiaomi-mimo-tp"
model = "mimo-v2-pro"
max_context_size = 131072
capabilities = ["thinking", "image_in"]
display_name = "MiMo V2 Pro (TP)"

[models."xiaomi-mimo-tp/mimo-v2-omni"]
provider = "xiaomi-mimo-tp"
model = "mimo-v2-omni"
max_context_size = 262144
capabilities = ["thinking", "image_in"]
display_name = "MiMo V2 Omni (TP)"

# --- FreeModel (free GPT models) ---
[models."freemodel/gpt-5.5"]
provider = "freemodel"
model = "gpt-5.5"
max_context_size = 1050000
capabilities = ["thinking", "image_in"]
display_name = "GPT-5.5 (FreeModel)"

[models."freemodel/gpt-5.4"]
provider = "freemodel"
model = "gpt-5.4"
max_context_size = 1050000
capabilities = ["thinking", "image_in"]
display_name = "GPT-5.4 (FreeModel)"

[models."freemodel/gpt-5.4-mini"]
provider = "freemodel"
model = "gpt-5.4-mini"
max_context_size = 1050000
capabilities = ["thinking", "image_in"]
display_name = "GPT-5.4 Mini (FreeModel)"

[models."freemodel/gpt-5.3-codex"]
provider = "freemodel"
model = "gpt-5.3-codex"
max_context_size = 1050000
capabilities = ["thinking", "image_in"]
display_name = "GPT-5.3 Codex (FreeModel)"

# --- Xunfei (讯飞星火) ---
[models."xunfei/astron-code-latest"]
provider = "xunfei"
model = "astron-code-latest"
max_context_size = 204800
capabilities = ["thinking"]
display_name = "Astron Code Latest (GLM-5.1)"

# --- Ainaiba (OpenAI Private) ---
[models."ainaiba/gpt-5.5"]
provider = "ainaiba"
model = "gpt-5.5"
max_context_size = 1050000
capabilities = ["thinking", "image_in"]
display_name = "GPT-5.5 (Private)"

[models."ainaiba/gpt-5.4"]
provider = "ainaiba"
model = "gpt-5.4"
max_context_size = 1050000
capabilities = ["thinking", "image_in"]
display_name = "GPT-5.4 (Private)"
```

---

## Notes

1. **API Keys:** Replace `YOUR_*_KEY_HERE` with actual keys. Kimi-code does NOT read shell environment variables for provider credentials — you must put the actual key values in the config file.

2. **Model Naming:** The `model` field (sent to the API) uses the same IDs as pi. The key (e.g., `deepseek/deepseek-v4-flash`) is the alias you'll use with `-m` flag.

3. **Capabilities:** Auto-detected by model name prefix in most cases. Explicit `capabilities` array only adds to the auto-detected set.

4. **Cost/Token Tracking:** Kimi-code doesn't have pi's `cost` or `thinkingLevelMap` fields. Cost tracking is handled differently.

5. **Thinking Levels:** Kimi-code uses `[thinking].effort` (low/medium/high/xhigh/max) globally, not per-model thinkingLevelMap.

6. **Duplicate Models:** Some models (e.g., `deepseek-v4-flash`) are available from both `deepseek` (direct) and `opencode-go` (subscription). Use the provider prefix to choose: `deepseek/deepseek-v4-flash` vs `opencode-go/deepseek-v4-flash`.
