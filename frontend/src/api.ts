const API_BASE = "/token-stats";

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

export async function fetchStats(
  from?: string,
  to?: string,
  source?: string,
  provider?: string,
  tzOffset?: number,
  resolution?: string
): Promise<StatsResponse> {
  const params = new URLSearchParams();
  if (from) params.set("from", from);
  if (to) params.set("to", to);
  if (source) params.set("source", source);
  if (provider) params.set("provider", provider);
  if (tzOffset !== undefined) params.set("tz_offset", String(tzOffset));
  if (resolution) params.set("resolution", resolution);
  const res = await fetch(`${API_BASE}/api/stats?${params}`);
  if (!res.ok) throw new Error("Failed to fetch stats");
  return res.json();
}

export async function fetchRequests(
  from?: string,
  to?: string,
  provider?: string,
  model?: string,
  source?: string,
  page: number = 1,
  limit: number = 50,
  tzOffset?: number
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
  const res = await fetch(`${API_BASE}/api/requests?${params}`);
  if (!res.ok) throw new Error("Failed to fetch requests");
  return res.json();
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

// ─── Unified quota response ───────────────────────────────────────────────────

export interface QuotaResponse {
  kimi: KimiQuotaStatus | null;
  opencode_go: OpenCodeQuotaStatus | null;
}

export async function fetchQuota(): Promise<QuotaResponse> {
  const res = await fetch(`${API_BASE}/api/quota`);
  if (!res.ok) throw new Error("Failed to fetch quota");
  return res.json();
}

export async function fetchFilters(): Promise<FilterOptions> {
  const res = await fetch(`${API_BASE}/api/filters`);
  if (!res.ok) throw new Error("Failed to fetch filters");
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

export interface XunfeiStatus {
  available: boolean;
  data: XunfeiStatusData | null;
  error: string | null;
}

export async function fetchXunfei(): Promise<XunfeiStatus> {
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
}

export interface SpecialPricing {
  xunfei_per_call: number;
  kimi_per_token: number;
  opencode_divisor: number;
}

export interface PricingConfig {
  usd_to_cny: number;
  rate_date: string;
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