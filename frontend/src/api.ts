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
}

export interface ModelStats {
  model: string;
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
  tzOffset?: number
): Promise<StatsResponse> {
  const params = new URLSearchParams();
  if (from) params.set("from", from);
  if (to) params.set("to", to);
  if (source) params.set("source", source);
  if (tzOffset !== undefined) params.set("tz_offset", String(tzOffset));
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

export async function fetchFilters(): Promise<FilterOptions> {
  const res = await fetch(`${API_BASE}/api/filters`);
  if (!res.ok) throw new Error("Failed to fetch filters");
  return res.json();
}