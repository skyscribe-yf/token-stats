import type { StatsResponse, PaginatedRequests } from "../api";

export function emptyStatsResponse(): StatsResponse {
  return {
    overall: {
      total_calls: 0,
      total_input_tokens: 0,
      total_output_tokens: 0,
      total_cache_read_tokens: 0,
      total_cache_write_tokens: 0,
      total_tokens: 0,
      total_cost: 0,
      avg_cache_hit_ratio: 0,
      weighted_cache_hit_ratio: 0,
    },
    by_vendor: [],
    by_date: [],
    by_model: [],
    by_source: [],
  };
}

export function emptyRequests(page: number): PaginatedRequests {
  return {
    data: [],
    total: 0,
    page,
    limit: 50,
    total_pages: 0,
  };
}
