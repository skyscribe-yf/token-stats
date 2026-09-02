const API_BASE = "/token-stats";

/** Provider merge map: aliases → canonical name */
const PROVIDER_MERGE: Record<string, string> = {
  "ollama-cloud": "ollama",
  // Yairouter is billed through the same platform as ainaba (AI奶爸),
  // and the Grok CLI records it as `yai-router`. Normalize both so the UI
  // shows one Yairouter vendor instead of splitting the data.
  "yai-router": "ainaba",
  "ainaiba": "ainaba",
  "xai": "ainaba",
  "yairouter": "ainaba",
};

/** Merge provider aliases in a StatsResponse so the UI shows unified vendors. */
function mergeProviders(data: StatsResponse): StatsResponse {
  const merge = <T extends { provider: string }>(
    items: T[],
    sumFields: (keyof T & string)[]
  ): T[] => {
    const map = new Map<string, T>();
    for (const item of items) {
      const canonical = PROVIDER_MERGE[item.provider] ?? item.provider;
      const key = "model" in item ? `${canonical}::${(item as any).model}` : canonical;
      const existing = map.get(key);
      if (existing && existing.provider !== item.provider) {
        // Merge into existing
        for (const f of sumFields) {
          (existing as any)[f] = ((existing as any)[f] as number) + ((item as any)[f] as number);
        }
      } else if (existing) {
        // Same provider, no merge needed (shouldn't happen)
      } else {
        map.set(key, { ...item, provider: canonical });
      }
    }
    return [...map.values()];
  };

  const numericVendorFields: (keyof VendorStats & string)[] = [
    "calls", "input_tokens", "output_tokens",
    "cache_read_tokens", "cache_write_tokens", "total_tokens", "cost",
  ];
  const numericModelFields: (keyof ModelStats & string)[] = [
    "calls", "input_tokens", "output_tokens",
    "cache_read_tokens", "cache_write_tokens", "total_tokens", "cost",
  ];

  data.by_vendor = merge(data.by_vendor, numericVendorFields);
  data.by_model = merge(data.by_model, numericModelFields);
  return data;
}

export interface AggregatedStats {
  total_calls: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cache_write_tokens: number;
  total_tokens: number;
  total_cost: number;
  avg_cache_hit_ratio: number;
  weighted_cache_hit_ratio: number;
}

export interface VendorStats {
  provider: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost: number;
  cache_hit_ratio: number;
  avg_ttft_ms: number;
  avg_tps: number;
}

export interface DateStats {
  date: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost: number;
  cache_hit_ratio: number;
  cache_hit_ratio_no_xunfei: number;
}

export interface SourceDetailStats {
  source: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost: number;
  cache_hit_ratio: number;
  avg_rpm: number;
  peak_rpm: number;
  avg_ttft_ms: number;
  avg_tps: number;
}

export interface ModelStats {
  model: string;
  provider: string;
  sources: string[];
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost: number;
  cache_hit_ratio: number;
  source_details: SourceDetailStats[];
  avg_rpm: number;
  peak_rpm: number;
  avg_ttft_ms: number;
  avg_tps: number;
}

export interface SourceStats {
  source: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost: number;
  cache_hit_ratio: number;
}

export interface StatsResponse {
  overall: AggregatedStats;
  by_vendor: VendorStats[];
  by_date: DateStats[];
  by_model: ModelStats[];
  by_source: SourceStats[];
}

export interface DetailedRequest {
  date: string;
  time: string;
  provider: string;
  model: string;
  source: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost: number;
  cache_hit_ratio: number;
  ttft_ms: number | null;
  tps: number | null;
}

export interface PaginatedRequests {
  data: DetailedRequest[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

export interface FilterOptions {
  vendors: string[];
  models: string[];
  sources: string[];
}

// ─── RPM Analysis ────────────────────────────────────────────────────────────

export interface MinuteBucket {
  minute: string;
  requests: number;
}

export interface ActiveWindow {
  start: string;
  end: string;
  duration_minutes: number;
  total_requests: number;
  avg_rpm: number;
  peak_rpm: number;
}

export interface RpmAnalysis {
  all_buckets: MinuteBucket[];
  windows: ActiveWindow[];
  overall_avg_rpm: number;
  overall_peak_rpm: number;
  total_active_minutes: number;
  gap_threshold_minutes: number;
}

export async function fetchStats(
  from?: string,
  to?: string,
  source?: string,
  provider?: string,
  tzOffset?: number,
  resolution?: string,
  model?: string
): Promise<StatsResponse> {
  const params = new URLSearchParams();
  if (from) params.set("from", from);
  if (to) params.set("to", to);
  if (source) params.set("source", source);
  if (provider) params.set("provider", provider);
  if (tzOffset !== undefined) params.set("tz_offset", String(tzOffset));
  if (resolution) params.set("resolution", resolution);
  if (model) params.set("model", model);
  const res = await fetch(`${API_BASE}/api/stats?${params}`);
  if (!res.ok) throw new Error("Failed to fetch stats");
  const data: StatsResponse = await res.json();
  return mergeProviders(data);
}

export async function fetchRequests(
  from?: string,
  to?: string,
  provider?: string,
  model?: string,
  source?: string,
  page: number = 1,
  limit: number = 50,
  tzOffset?: number,
  showZeroTokens?: boolean
): Promise<PaginatedRequests> {
  const params = new URLSearchParams();
  if (from) params.set("from", from);
  if (to) params.set("to", to);
  if (provider) params.set("provider", provider);
  if (model) params.set("model", model);
  if (source) params.set("source", source);
  params.set("page", String(page));
  params.set("limit", String(limit));
  if (tzOffset !== undefined) params.set("tz_offset", String(tzOffset));
  if (showZeroTokens !== undefined) params.set("show_zero_tokens", String(showZeroTokens));
  const res = await fetch(`${API_BASE}/api/requests?${params}`);
  if (!res.ok) throw new Error("Failed to fetch requests");
  const data: PaginatedRequests = await res.json();
  // Merge provider aliases in request rows
  for (const r of data.data) {
    if (PROVIDER_MERGE[r.provider]) r.provider = PROVIDER_MERGE[r.provider];
  }
  return data;
}

// ─── Kimi Code Quota ──────────────────────────────────────────────────────────

export interface QuotaKimiCode {
  provider: string;
  weekly_limit: number;
  weekly_used: number;
  weekly_remaining: number;
  weekly_reset_time: string | null;
  rp5h_limit: number;
  rp5h_used: number;
  rp5h_remaining: number;
  rp5h_reset_time: string | null;
  total_limit: number;
  total_remaining: number;
  parallel_limit: number;
  membership_level: string | null;
  sub_type: string | null;
}

export interface KimiQuotaStatus {
  available: boolean;
  data: QuotaKimiCode | null;
  error: string | null;
}

// ─── OpenCode-go Quota ────────────────────────────────────────────────────────

export interface QuotaOpenCodeUsageEntry {
  usage_type: string;
  percentage: number;
  resets_in: string;
  reset_at: string | null;
}

export interface QuotaOpenCode {
  provider: string;
  entries: QuotaOpenCodeUsageEntry[];
  workspace_url: string | null;
}

export interface OpenCodeQuotaStatus {
  available: boolean;
  data: QuotaOpenCode | null;
  error: string | null;
}

// ─── CodeBuddy (codebuddy.cn) Quota ──────────────────────────────────────────

export interface CodeBuddyPackage {
  package_code: string;
  package_name: string;
  is_subscription: boolean;
  unit: string;
  total: number;
  used: number;
  remain: number;
  cycle_start: string | null;
  cycle_end: string | null;
}

export interface CodeBuddyQuotaData {
  is_paid_user: boolean;
  packages: CodeBuddyPackage[];
}

export interface CodeBuddyQuotaStatus {
  available: boolean;
  data: CodeBuddyQuotaData | null;
  error: string | null;
}

// ─── Xiaomi MiMo TP Quota ────────────────────────────────────────────────────

export interface XiaomiMiMoUsageEntry {
  name: string;
  used: number;
  limit: number;
  percent: number;
}

export interface XiaomiMiMoQuotaData {
  entries: XiaomiMiMoUsageEntry[];
  month_percent: number;
  plan_name: string;
  plan_code: string;
  current_period_end: string | null;
  expired: boolean;
  enable_auto_renew: boolean;
}

export interface XiaomiMiMoQuotaStatus {
  available: boolean;
  data: XiaomiMiMoQuotaData | null;
  error: string | null;
}

// ─── Unified quota response ───────────────────────────────────────────────────

export interface QuotaResponse {
  kimi: KimiQuotaStatus | null;
  kimi_ex: KimiQuotaStatus | null;
  opencode_go: OpenCodeQuotaStatus | null;
  opencode_go_ex: OpenCodeQuotaStatus | null;
  xiaomi_mimo: XiaomiMiMoQuotaStatus | null;
  commandcode: CommandCodeQuotaStatus | null;
  commandcode_ex: CommandCodeQuotaStatus | null;
  codebuddy: CodeBuddyQuotaStatus | null;
  ollama: OllamaQuotaStatus | null;
  meituan: MeituanQuotaStatus | null;
  fenno: FennoQuotaStatus | null;
  fenno_ex: FennoQuotaStatus | null;
  grok: GrokQuotaStatus | null;
  dimagent: DimAgentQuotaStatus | null;
}

// ─── DimAgent Subscription Quota ────────────────────────────────────────────

export interface DimAgentFeatureMeter {
  feature_key: string;
  unit: string;
  unlimited: boolean;
  used: number;
  allowance: number;
  remaining: number;
  period_end: string | null;
}

export interface DimAgentRecent30d {
  calls: number;
  total_tokens: number;
  prompt_tokens: number;
  completion_tokens: number;
  cache_tokens: number;
  quota_units: number;
}

export interface DimAgentQuotaData {
  plan_name: string;
  plan_description: string | null;
  price_cny: number;
  billing_interval: string;
  subscription_status: string;
  cancel_at_period_end: boolean;
  period_start: string;
  period_end: string;
  total_units: number;
  used_units: number;
  remaining_units: number;
  estimated_remaining_calls: number | null;
  request_count_total: number;
  feature_meters: DimAgentFeatureMeter[];
  recent_30d: DimAgentRecent30d | null;
}

export interface DimAgentQuotaStatus {
  available: boolean;
  data: DimAgentQuotaData | null;
  error: string | null;
}

// ─── Ollama Quota ───────────────────────────────────────────────────────────

export interface OllamaUsageEntry {
  usage_type: string;
  percentage: number;
  reset_time: string | null;
}

export interface OllamaQuotaData {
  plan_name: string;
  renews_on: string | null;
  price: string | null;
  usage_entries: OllamaUsageEntry[];
  has_annual_option: boolean;
  has_max_upgrade: boolean;
  estimated_tokens_used: number | null;
  estimated_cost_cny: number | null;
}

export interface OllamaQuotaStatus {
  available: boolean;
  data: OllamaQuotaData | null;
  error: string | null;
}

// ─── Meituan LongCat Quota ──────────────────────────────────────────────────

export interface MeituanTokenPack {
  package_name: string;
  source_type_text: string;
  source_type_code: number;
  status_text: string;
  status_code: number;
  total_token_amount: number;
  used_token_amount: number;
  remain_token_amount: number;
  usage_percent: number;
  valid_start_time: string;
  valid_end_date_text: string;
  applicable_models: string[];
}

export interface MeituanQuotaData {
  packs: MeituanTokenPack[];
  active_count: number;
  recent_7d_tokens: number;
}

export interface MeituanQuotaStatus {
  available: boolean;
  data: MeituanQuotaData | null;
  error: string | null;
}

// ─── Fenno Subscription Quota ───────────────────────────────────────────────

export interface FennoSubscriptionGroup {
  name: string;
  platform: string;
  daily_limit_usd: number | null;
  weekly_limit_usd: number | null;
  monthly_limit_usd: number | null;
}

export interface FennoSubscription {
  status: string;
  expires_at: string | null;
  daily_usage_usd: number;
  weekly_usage_usd: number;
  monthly_usage_usd: number;
  daily_window_start: string | null;
  weekly_window_start: string | null;
  monthly_window_start: string | null;
  group: FennoSubscriptionGroup;
}

export interface FennoQuotaData {
  subscriptions: FennoSubscription[];
}

export interface FennoQuotaStatus {
  available: boolean;
  data: FennoQuotaData | null;
  error: string | null;
}

// ─── Grok / XAI Quota ────────────────────────────────────────────────────────

export interface GrokQuotaData {
  user_id: string;
  team_id: string;
  zdr_status: string;
  total_calls: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_tokens: number;
  estimated_cost_cny: number;
  weekly_usage_percent?: number;
  weekly_remaining_percent?: number;
  weekly_period_start?: string;
  weekly_reset_at?: string | null;
  weekly_breakdown?: GrokQuotaBreakdown[];
}

export interface GrokQuotaBreakdown {
  product: string;
  usage_percent: number;
}

export interface GrokQuotaStatus {
  available: boolean;
  data: GrokQuotaData | null;
  error: string | null;
}

// ─── CommandCode Quota ───────────────────────────────────────────────────────

export interface CommandCodeWindowLimit {
  used: number;
  cap: number;
  reset_at: string | null;
}

export interface CommandCodeQuotaData {
  plan_name: string;
  subscription_status: string;
  cancel_at_period_end: boolean | null;
  user_name: string;
  user_id: string;
  monthly_credits_total: number | null;
  monthly_credits_used: number;
  monthly_credits_remaining: number;
  purchased_credits: number;
  premium_monthly_credits: number;
  opensource_monthly_credits: number;
  current_period_end: string | null;
  total_requests: number;
  total_tokens: number;
  total_tokens_in: number;
  total_tokens_out: number;
  five_hour: CommandCodeWindowLimit | null;
  weekly: CommandCodeWindowLimit | null;
}

export interface CommandCodeQuotaStatus {
  available: boolean;
  data: CommandCodeQuotaData | null;
  error: string | null;
}

export async function fetchQuota(): Promise<QuotaResponse> {
  const res = await fetch(`${API_BASE}/api/quota`);
  if (!res.ok) throw new Error("Failed to fetch quota");
  return res.json();
}

export async function fetchFilters(): Promise<FilterOptions> {
  const res = await fetch(`${API_BASE}/api/filters`);
  if (!res.ok) throw new Error("Failed to fetch filters");
  const data: FilterOptions = await res.json();
  // Merge provider aliases in vendor list
  data.vendors = [...new Set(data.vendors.map(v => PROVIDER_MERGE[v] ?? v))];
  return data;
}

export async function fetchRpm(
  from?: string,
  to?: string,
  source?: string,
  provider?: string,
  tzOffset?: number,
  gapThreshold?: number,
  model?: string
): Promise<RpmAnalysis> {
  const params = new URLSearchParams();
  if (from) params.set("from", from);
  if (to) params.set("to", to);
  if (source) params.set("source", source);
  if (provider) params.set("provider", provider);
  if (tzOffset !== undefined) params.set("tz_offset", String(tzOffset));
  if (gapThreshold !== undefined) params.set("gap_threshold", String(gapThreshold));
  if (model) params.set("model", model);
  const res = await fetch(`${API_BASE}/api/rpm?${params}`);
  if (!res.ok) throw new Error("Failed to fetch RPM analysis");
  return res.json();
}

// ─── TPS Time-Series Analysis ───────────────────────────────────────────────

export interface TpsDataPoint {
  time: string;
  tps: number;
}

export interface TpsModelSeries {
  model: string;
  provider: string;
  data_points: TpsDataPoint[];
}

export interface TpsAnalysis {
  models: TpsModelSeries[];
  available_models: string[];
}

export async function fetchTps(
  from?: string,
  to?: string,
  source?: string,
  provider?: string,
  tzOffset?: number,
  model?: string,
  models?: string
): Promise<TpsAnalysis> {
  const params = new URLSearchParams();
  if (from) params.set("from", from);
  if (to) params.set("to", to);
  if (source) params.set("source", source);
  if (provider) params.set("provider", provider);
  if (tzOffset !== undefined) params.set("tz_offset", String(tzOffset));
  if (model) params.set("model", model);
  if (models) params.set("models", models);
  const res = await fetch(`${API_BASE}/api/tps?${params}`);
  if (!res.ok) throw new Error("Failed to fetch TPS analysis");
  return res.json();
}

// ─── Ainaiba (XAI) Credit ───────────────────────────────────────────────────

export interface AinaibaCreditCard {
  amount: number;
  balance: number;
  expires_at: string;
}

export interface AinaibaCreditData {
  user_id: number;
  name: string;
  email: string;
  alias: string;
  balance: number;
  credit_total: number;
  credit_used: number;
  expires_at: string;
  cards: AinaibaCreditCard[];
  total_requests: number;
  daily_used: number;
  daily_requests: number;
  daily_input_tokens: number;
  daily_output_tokens: number;
  daily_reasoning_tokens: number;
  daily_cached_tokens: number;
  monthly_used: number;
  monthly_requests: number;
  monthly_input_tokens: number;
  monthly_output_tokens: number;
  monthly_reasoning_tokens: number;
  monthly_cached_tokens: number;
  hard_limit: number;
  daily_limit: number;
  rpm: number;
  rph: number;
  rpd: number;
}

export interface AinaibaCreditResponse {
  available: boolean;
  data: AinaibaCreditData | null;
  error: string | null;
}

export async function fetchAinaibaCredit(): Promise<AinaibaCreditResponse> {
  const res = await fetch(`${API_BASE}/api/ainaiba-credit`);
  if (!res.ok) throw new Error("Failed to fetch Ainaiba credit");
  return res.json();
}

// ─── Xunfei (iFlytek) Coding Plan ─────────────────────────────────────────────

export interface XunfeiUsage {
  package_used: number;
  package_limit: number;
  package_left: number;
  rp5h_used: number;
  rp5h_limit: number;
  rpw_used: number;
  rpw_limit: number;
}

export interface XunfeiBalance {
  cash: number;
  virtual_balance: number;
}

export interface XunfeiModelInfo {
  model_id: string;
  name: string;
  context_length: string;
  is_default: boolean;
}

export interface XunfeiStatusData {
  plan_name: string;
  package_id: number;
  status: string;
  expires_at: string;
  created_at: string;
  price: number;
  usage: XunfeiUsage;
  balance: XunfeiBalance;
  app_id: string;
  api_key_masked: string;
  model_list: XunfeiModelInfo[];
}

export interface XunfeiAccountStatus {
  label: string;
  available: boolean;
  data: XunfeiStatusData[];
  error: string | null;
}

export interface XunfeiMultiStatus {
  accounts: XunfeiAccountStatus[];
}

export async function fetchXunfei(): Promise<XunfeiMultiStatus> {
  const res = await fetch(`${API_BASE}/api/xunfei`);
  if (!res.ok) throw new Error("Failed to fetch xunfei status");
  return res.json();
}

// ─── Backup / Restore ────────────────────────────────────────────────────────

export interface RestoreResponse {
  success: boolean;
  before_count: number;
  after_count: number;
  added: number;
  skipped: number;
  errors: string[];
}

export interface StoreInfo {
  enabled: boolean;
  db_path: string;
  db_records: number;
  memory_records: number;
  pending_records: number;
  db_size_bytes: number;
}

export async function fetchStoreInfo(): Promise<StoreInfo> {
  const res = await fetch(`${API_BASE}/api/store/info`);
  if (!res.ok) throw new Error("Failed to fetch token store info");
  return res.json();
}

export async function restoreFromStore(): Promise<RestoreResponse> {
  const res = await fetch(`${API_BASE}/api/store/restore`, { method: "POST" });
  if (!res.ok) throw new Error("Failed to restore from token store");
  return res.json();
}

export async function fetchRefresh(): Promise<{ success: boolean; added: number; total: number }> {
  const res = await fetch(`${API_BASE}/api/refresh`, { method: "POST" });
  if (!res.ok) throw new Error("Failed to refresh data");
  return res.json();
}

// ─── Pricing Config ───────────────────────────────────────────────────────────

export interface ModelPriceConfig {
  name: string;
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  tier_threshold?: number;
}

export interface AinabaSegment {
  before?: string;
  divisor: number;
}

export interface KimiApiModelPrice {
  name: string;
  input: number;
  cache_read: number;
  output: number;
}

export interface SpecialPricing {
  xunfei_per_call: number;
  codebuddy_usd_per_credit: number;
  kimi_per_token: number;
  kimi_subscription_multiplier: number;
  kimi_api_models: KimiApiModelPrice[];
  xiaomi_mimo_tp_per_token: number;
  opencode_divisor: number;
  ainaba_divisor: number;
  ainaba_segments: AinabaSegment[];
  freemodel_divisor: number;
  commandcode_divisor: number;
}

export interface PricingConfig {
  usd_to_cny: number;
  rate_date: string;
  usd_to_cny_segments: { effective_from: string | null; rate: number }[];
  special: SpecialPricing;
  model: ModelPriceConfig[];
}

export async function fetchPricing(): Promise<PricingConfig> {
  const res = await fetch(`${API_BASE}/api/pricing`);
  if (!res.ok) throw new Error("Failed to fetch pricing");
  return res.json();
}

export async function reloadPricing(): Promise<{ success: boolean }> {
  const res = await fetch(`${API_BASE}/api/pricing/reload`, { method: "POST" });
  if (!res.ok) throw new Error("Failed to reload pricing");
  return res.json();
}

export async function exportBackup(): Promise<Response> {
  const res = await fetch(`${API_BASE}/api/export`);
  if (!res.ok) throw new Error("Failed to export backup");
  return res;
}

export async function restoreBackup(backupDir: string): Promise<RestoreResponse> {
  const res = await fetch(`${API_BASE}/api/restore`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ backup_dir: backupDir }),
  });
  if (!res.ok) throw new Error("Failed to restore backup");
  return res.json();
}

// ─── Advanced Models Settings ─────────────────────────────────────────────────

export async function fetchAdvancedModels(): Promise<string[]> {
  const res = await fetch(`${API_BASE}/api/settings/advanced-models`);
  if (!res.ok) throw new Error("Failed to fetch advanced models");
  return res.json();
}

export async function saveAdvancedModels(models: string[]): Promise<{ success: boolean }> {
  const res = await fetch(`${API_BASE}/api/settings/advanced-models`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(models),
  });
  if (!res.ok) throw new Error("Failed to save advanced models");
  return res.json();
}

// ─── Subscription Settings ─────────────────────────────────────────────────

export interface SubscriptionSettings {
  kimi_monthly_start_day: number | null;
  kimi_ex_monthly_start_day: number | null;
  kimi_subscription_multiplier: number;
  grok_divisor: number;
}

export async function fetchSubscriptionSettings(): Promise<SubscriptionSettings> {
  const res = await fetch(`${API_BASE}/api/settings/subscriptions`);
  if (!res.ok) throw new Error("Failed to fetch subscription settings");
  return res.json();
}

export async function saveSubscriptionSettings(settings: SubscriptionSettings): Promise<void> {
  const res = await fetch(`${API_BASE}/api/settings/subscriptions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(settings),
  });
  if (!res.ok) throw new Error("Failed to save subscription settings");
  const data = await res.json();
  if (!data.success) throw new Error(data.error || "Failed to save subscription settings");
}
