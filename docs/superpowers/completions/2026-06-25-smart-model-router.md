# Smart Model Router Completion Note

Global Pi skill deployment lives at `/home/skyscribe/.pi/agent/skills/smart-model-router`.

## What Was Verified

- Added and passed regression coverage for unknown-provider fallback health defaults.
- Added and passed regression coverage for invalid model-ref rejection in `applySettingsPatch()`.
- Added and passed regression coverage for the CLI main-module guard via `isMainModule()`.
- Updated `model-router.mjs` so unknown providers now use the documented `quota unknown` fallback with `headroom: 0.5`.
- Updated the CLI entry-point guard to use filesystem-path comparison via `fileURLToPath()`.
- Removed the stray unused `evaluateOpenCodeQuota()` argument and made the `/api/stats` and `/api/pricing` fetches explicit.
- Re-ran the smart-model-router unit tests successfully.
- Re-ran live dry-run smoke checks for `--trigger plan` and `--trigger review` successfully.
- Confirmed dry-run mode does not modify `/home/skyscribe/.pi/agent/settings.json`.

## Verification Commands

```bash
cd /home/skyscribe/.pi/agent/skills/smart-model-router && node --test tests/model-router.test.mjs
python3 - <<'PY'
from pathlib import Path
import json, re, subprocess
settings = Path('/home/skyscribe/.pi/agent/settings.json')
before = settings.stat().st_mtime_ns
models = subprocess.check_output(['pi', '--list-models'], text=True)
refs = set()
for line in models.splitlines()[1:]:
    parts = line.split()
    if len(parts) >= 2:
        refs.add(f'{parts[0]}/{parts[1]}')
plan_out = subprocess.check_output(['node', '/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs', '--trigger', 'plan', '--json'], text=True)
review_out = subprocess.check_output(['node', '/home/skyscribe/.pi/agent/skills/smart-model-router/scripts/model-router.mjs', '--trigger', 'review'], text=True)
after = settings.stat().st_mtime_ns
plan = json.loads(plan_out)
assert list(plan['settingsPatch'].keys()) == ['subagents']
assert list(plan['settingsPatch']['subagents'].keys()) == ['agentOverrides']
for rec in plan['recommendations'].values():
    assert rec['model'] in refs
    for ref in rec.get('fallbackModels', []):
        assert ref in refs
secret_patterns = [r'@[A-Za-z0-9.-]+', r'auth=', r'cookie=', r'loginUrl=', r'callback=', r'api[_-]?key', r'access_token']
assert not any(re.search(pattern, review_out, re.I) for pattern in secret_patterns)
assert before == after
print('live_smoke_ok')
PY
```

## Note

The repo tracks this completion note and the implementation plan. The deployable skill files themselves are global Pi assets outside this repository.
