import type pg from "pg";

export type SortKey = "relevance" | "newest" | "largest" | "name";
export type SortOrder = "asc" | "desc";

export interface SearchParams {
  q: string;
  sizeMin?: number;
  sizeMax?: number;
  fileMin?: number;
  ageMin?: number;
  sort: SortKey;
  order: SortOrder;
  from: number;
  limit: number;
}

export interface SearchHit {
  info_hash: string;
  name: string;
  size_bytes: number | null;
  file_count: number | null;
  first_seen: number;
  last_seen: number;
  similarity: number | null;
}

export interface SearchResult {
  query: string;
  total: number;
  from: number;
  limit: number;
  data: SearchHit[];
}

/**
 * Instant fuzzy search over torrent names using pg_trgm similarity ranking,
 * with filters and keyset/offset pagination. All parameterized — the query
 * string is bound, never interpolated.
 */
export class SearchService {
  constructor(private readonly pool: pg.Pool) {}

  async search(params: SearchParams): Promise<SearchResult> {
    // Data query: $1 = q (similarity), then filters, then limit/offset.
    // Count query: no similarity column, so filters start at $1.
    const dataBinds: unknown[] = [params.q];
    const filterBinds: unknown[] = [];
    const whereData: string[] = ["1=1"];
    const whereCount: string[] = ["1=1"];

    const pushFilter = (col: string, op: string, value: number): void => {
      // Count binds start at 1; data binds start at 2 (after q).
      filterBinds.push(value);
      const countIdx = filterBinds.length;
      whereCount.push(`${col} ${op} $${countIdx}`);
      const dataIdx = filterBinds.length + 1;
      whereData.push(`${col} ${op} $${dataIdx}`);
      dataBinds.push(value);
    };

    if (params.sizeMin !== undefined) {
      pushFilter("size_bytes", ">=", params.sizeMin);
    }
    if (params.sizeMax !== undefined) {
      pushFilter("size_bytes", "<=", params.sizeMax);
    }
    if (params.fileMin !== undefined) {
      pushFilter("file_count", ">=", params.fileMin);
    }
    if (params.ageMin !== undefined) {
      pushFilter("first_seen", ">=", params.ageMin);
    }

    const simExpr = "similarity(name, $1)";
    const orderBy = this.orderBy(params.sort, params.order, simExpr);

    const countSql = `
      SELECT COUNT(*)::int AS total
      FROM torrents
      WHERE ${whereCount.join(" AND ")}
    `;
    const { rows: countRows } = await this.pool.query<{ total: number }>(
      countSql,
      filterBinds,
    );
    const total = countRows[0]?.total ?? 0;

    dataBinds.push(params.limit);
    const limitIdx = dataBinds.length;
    dataBinds.push(params.from);
    const fromIdx = dataBinds.length;

    const dataSql = `
      SELECT encode(info_hash, 'hex') AS info_hash,
             name, size_bytes, file_count, first_seen, last_seen,
             ${simExpr} AS similarity
      FROM torrents
      WHERE ${whereData.join(" AND ")}
      ORDER BY ${orderBy}
      LIMIT $${limitIdx} OFFSET $${fromIdx}
    `;
    const { rows } = await this.pool.query<SearchHit>(dataSql, dataBinds);

    return {
      query: params.q,
      total,
      from: params.from,
      limit: params.limit,
      data: rows,
    };
  }

  private orderBy(sort: SortKey, order: SortOrder, simExpr: string): string {
    const dir = order === "asc" ? "ASC" : "DESC";
    switch (sort) {
      case "relevance":
        return `${simExpr} ${dir}, last_seen DESC`;
      case "newest":
        return `first_seen ${dir}`;
      case "largest":
        return `size_bytes ${dir}`;
      case "name":
        return `name ${dir}`;
    }
  }
}
