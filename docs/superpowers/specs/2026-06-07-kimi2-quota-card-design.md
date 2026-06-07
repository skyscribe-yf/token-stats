# Kimi2 Quota Card — Design Spec

## Goal
Point the existing **Kimi (EX)** quota balance card at the second `kimi2` account instead of the legacy `kimi-code-ex.json` credentials.

## Background
The dashboard already supports two Kimi Code quota slots:
- `kimi` — primary account (`~/.kimi/credentials/kimi-code.json`)
- `kimi_ex` — secondary account (`~/.kimi/credentials/kimi-code-ex.json`)

The user’s second account is aliased as `kimi2` and lives in a separate home-style directory (`~/.kimi-code-user2`). They want the existing `kimi_ex` card to render this account’s quota, without adding a third card.

## Architecture

### Backend
**Single-file change:** `backend/src/quota/kimi.rs`

- Modify `get_kimi_credentials_path_ex()`:
  - Keep the `KIMI_CREDENTIALS_PATH_EX` env override for flexibility.
  - Change the default fallback path from:
    ```
    ~/.kimi/credentials/kimi-code-ex.json
    ```
    to:
    ```
    ~/.kimi-code-user2/credentials/kimi-code.json
    ```

- No changes to:
  - Token refresh logic (`refresh_token`)
  - API endpoint (`/api/quota`)
  - Response types (`QuotaResponse`, `KimiQuotaStatus`, etc.)

### Frontend
No changes. The existing `KimiCard` rendered with `suffix="EX"` continues to consume `quota.kimi_ex`.

### Data Flow
```
kimi2 CLI writes credentials ──► ~/.kimi-code-user2/credentials/kimi-code.json
                                        │
                                        ▼
                         backend/src/quota/kimi.rs
                         get_kimi_credentials_path_ex()
                                        │
                                        ▼
                              GET /usages (Kimi API)
                                        │
                                        ▼
                           quota.kimi_ex in QuotaResponse
                                        │
                                        ▼
                           Frontend KimiCard (suffix="EX")
```

## Error Handling
Existing behavior is preserved:
- Missing file → `available: false` with an error message.
- Expired token → automatic refresh via `refresh_token()`.
- Failed refresh → returns an error prompting login.

## Testing
- Verify the dashboard loads and the **Kimi Code (EX)** card shows the `kimi2` account’s weekly / 5h quotas.
- If the card still shows the old data, check that `~/.kimi-code-user2/credentials/kimi-code.json` exists and contains a valid access token.

## Assumptions
- The `kimi2` account’s credential layout mirrors the primary account, but rooted at `~/.kimi-code-user2` instead of `~/.kimi`.
- The user does **not** need token-usage records from `kimi2` sessions loaded into the dashboard (quota-only fix).
