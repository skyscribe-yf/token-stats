# Kimi Subscription Cost Estimation

## Goal

Replace the flat Kimi subscription per-token estimate with a model-aware API-equivalent calculation, discounted by a user-configurable subscription multiplier. The initial multiplier is 20.

## Scope

The calculation applies to zero-stored-cost records whose canonical provider is `kimi`. This retains the current scope: Kimi CLI and Kimi Code records, including records whose provider was normalized through vendor merging. Records with a stored cost keep their existing handling.

## Pricing

Rates are CNY per 1M tokens. Cache writes are free.

| Model | Input | Cache hit | Output |
| --- | ---: | ---: | ---: |
| Kimi K3 | 20 | 2 | 100 |
| Kimi K2.6 | 6.5 | 1.1 | 27 |
| Kimi K2.7 Code | 6.5 | 1.3 | 27 |

The backend will calculate:

`(input * input_rate + cache_read * cache_hit_rate + output * output_rate) / 1_000_000 / multiplier`

Unknown Kimi model names will use the K2.7 Code rate, matching the existing Kimi model-price fallback behavior. `cache_write_tokens` will not contribute to Kimi API-equivalent cost.

## Architecture

1. Add Kimi CNY-per-million model rates to the pricing configuration, separate from the existing USD model table so non-Kimi calculations remain unchanged.
2. Add `kimi_subscription_multiplier` to persisted subscription settings, defaulting to `20`, and validate it as a positive finite number.
3. Initialize and update the in-memory pricing state from this setting so aggregation does not repeatedly read the settings file.
4. Replace the Kimi flat per-token branch in `display_cost` with model-rate resolution and the formula above.
5. Add a settings-drawer number field, saved through the existing subscription-settings endpoint, and replace the obsolete Kimi per-token display copy with the model-aware formula and active multiplier.

## Error Handling and Compatibility

Existing settings files omit the new field and deserialize to the default multiplier. Invalid submitted multipliers are rejected without persisting changes. Invalid values in a hand-edited settings file revert to the default. Existing pricing configuration snippets continue to parse through serde defaults.

## Testing

Backend tests will verify K3, K2.6, K2.7 Code, cache-write exclusion, fallback pricing, multiplier updates, and validation/default behavior. Frontend typechecking and production build will validate the setting UI and API contract.

## Sources

- https://platform.kimi.com/docs/pricing/chat-k3
- https://platform.kimi.com/docs/pricing/chat-k26
- https://platform.kimi.com/docs/pricing/chat-k27-code
