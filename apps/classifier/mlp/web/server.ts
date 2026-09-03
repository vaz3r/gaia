import express from "express";
import cors from "cors";
import path from "node:path";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { spawn, type ChildProcess } from "node:child_process";
import pg from "pg";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const { Pool } = pg;

// --- Database ---
const pool = new Pool({
  host: process.env.DB_HOST || "workspace-production",
  port: parseInt(process.env.DB_PORT || "5432", 10),
  user: process.env.DB_USER || "crawler",
  database: process.env.DB_NAME || "craw",
  password:
    process.env.DB_PASSWORD ||
    "83fec11c363e2e90cbea2a0303ace95a8b5d4bbaf897fc97f49195ffbbf7978b",
  max: 10,
  idleTimeoutMillis: 30000,
  connectionTimeoutMillis: 10000,
});

async function query(text: string, params?: unknown[]) {
  return pool.query(text, params);
}

// --- Python child process ---
let pythonProcess: ChildProcess | null = null;
let pythonReady = false;
let pythonQueue: {
  resolve: (result: unknown) => void;
  reject: (err: Error) => void;
}[] = [];
let requestId = 0;

function startPython(): void {
  const pyPath = path.join(__dirname, "python", "classify.py");
  const venvPython = path.join(__dirname, "..", "..", "deepseek", "venv", "bin", "python3");
  // Use venv python if it exists, otherwise fall back to system python3
  const pythonBin = fs.existsSync(venvPython) ? venvPython : "python3";
  console.log(`Starting Python: ${pythonBin} ${pyPath}`);

  pythonProcess = spawn(pythonBin, [pyPath], {
    stdio: ["pipe", "pipe", "pipe"],
    cwd: path.join(__dirname, ".."),
  });

  let buffer = "";

  pythonProcess.stdout!.on("data", (data: Buffer) => {
    buffer += data.toString();
    const lines = buffer.split("\n");
    buffer = lines.pop()!;

    for (const line of lines) {
      if (!line.trim()) continue;
      try {
        const msg = JSON.parse(line);
        if (msg.status === "ready") {
          pythonReady = true;
          console.log("Python classifier ready");
        } else {
          const pending = pythonQueue.shift();
          if (pending) {
            if (msg.status === "error") {
              pending.reject(new Error(msg.error));
            } else {
              pending.resolve(msg);
            }
          }
        }
      } catch {
        // ignore parse errors for partial lines
      }
    }
  });

  pythonProcess.stderr!.on("data", (data: Buffer) => {
    const msg = data.toString().trim();
    if (msg) console.error("[python]", msg);
  });

  pythonProcess.on("exit", (code) => {
    console.error(`Python exited with code ${code}`);
    pythonReady = false;
    pythonProcess = null;
    // Reject all pending
    for (const p of pythonQueue) {
      p.reject(new Error("Python process exited"));
    }
    pythonQueue = [];
    // Restart after delay
    setTimeout(startPython, 2000);
  });
}

function classifyTorrent(torrent: Record<string, unknown>): Promise<unknown> {
  return new Promise((resolve, reject) => {
    if (!pythonReady || !pythonProcess) {
      return reject(new Error("Python classifier not ready"));
    }
    pythonQueue.push({ resolve, reject });
    pythonProcess.stdin!.write(JSON.stringify(torrent) + "\n");
  });
}

// --- SQL helpers ---
function escapeLike(s: string) {
  return s.replace(/[\\%_]/g, (c) => "\\" + c);
}

const TORRENT_COLUMNS = `
  encode(t.infohash, 'hex') AS infohash,
  t.name,
  t.file_count,
  t.total_size,
  CASE
    WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN (
      SELECT array_agg(DISTINCT ext) FROM (
        SELECT lower(split_part(elem->'path'->>-1, '.', -1)) AS ext
        FROM jsonb_array_elements(t.files) AS elem
      ) sub WHERE ext IS NOT NULL AND ext != '' LIMIT 10
    )
    ELSE NULL
  END AS extensions,
  CASE
    WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN (
      SELECT array_agg(DISTINCT folder) FROM (
        SELECT elem->'path'->>0 AS folder
        FROM jsonb_array_elements(t.files) AS elem
        WHERE jsonb_array_length(elem->'path') > 1
      ) sub WHERE folder IS NOT NULL LIMIT 10
    )
    ELSE NULL
  END AS top_folders,
  CASE
    WHEN t.files IS NOT NULL AND jsonb_array_length(t.files) > 0 THEN (
      SELECT jsonb_agg(jsonb_build_object('name', sub.elem->'path'->>-1, 'size', sub.elem->'length'))
      FROM (
        SELECT elem FROM jsonb_array_elements(t.files) AS elem
        ORDER BY (elem->'length')::bigint DESC LIMIT 3
      ) sub
    )
    ELSE NULL
  END AS largest_files
`;

// --- Express app ---
const app = express();
app.use(cors());
app.use(express.json());

// Serve static files in production
const distDir = path.join(__dirname, "client", "dist");
app.use(express.static(distDir));

// GET /api/torrents?search=&page=&limit=
app.get("/api/torrents", async (req, res) => {
  try {
    const search = ((req.query.search as string) || "").trim();
    const page = Math.max(1, parseInt(req.query.page as string, 10) || 1);
    const limit = Math.min(50, Math.max(1, parseInt(req.query.limit as string, 10) || 20));
    const offset = (page - 1) * limit;

    let where = "";
    const params: unknown[] = [];

    if (search.length > 0) {
      params.push(`%${escapeLike(search)}%`);
      where = `WHERE name ILIKE $1`;
    }

    const countRes = await query(`SELECT count(*) AS total FROM torrents ${where}`, params);
    const total = parseInt(countRes.rows[0].total, 10);

    const rowsRes = await query(
      `SELECT encode(infohash, 'hex') AS infohash, name, file_count, total_size
       FROM torrents ${where} ORDER BY verified_at DESC LIMIT ${limit} OFFSET ${offset}`,
      params
    );

    res.json({ data: rowsRes.rows, page, limit, total });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error("GET /api/torrents error:", msg);
    res.status(500).json({ error: msg });
  }
});

// GET /api/torrents/random?count=10
app.get("/api/torrents/random", async (req, res) => {
  try {
    const count = Math.min(50, Math.max(1, parseInt(req.query.count as string, 10) || 10));

    const rowsRes = await query(
      `SELECT
        encode(infohash, 'hex') AS infohash,
        name,
        file_count,
        total_size
       FROM torrents
       WHERE file_count > 1
       ORDER BY random()
       LIMIT ${count}`
    );

    res.json({ data: rowsRes.rows });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error("GET /api/torrents/random error:", msg);
    res.status(500).json({ error: msg });
  }
});

// POST /api/classify
app.post("/api/classify", async (req, res) => {
  try {
    const { infohash } = req.body;
    if (!infohash) {
      return res.status(400).json({ error: "Missing torrent infohash" });
    }

    // Fetch full torrent details from DB
    const detailRes = await query(
      `SELECT ${TORRENT_COLUMNS}
       FROM torrents t
       WHERE encode(t.infohash, 'hex') = $1`,
      [infohash]
    );

    if (detailRes.rows.length === 0) {
      return res.status(404).json({ error: "Torrent not found" });
    }

    const torrent = detailRes.rows[0];
    const result = await classifyTorrent(torrent as unknown as Record<string, unknown>);
    res.json(result);
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error("POST /api/classify error:", msg);
    res.status(500).json({ error: msg });
  }
});

// GET /api/stats
app.get("/api/stats", async (_req, res) => {
  try {
    const totalRes = await query("SELECT count(*) AS total FROM torrents");
    const labeledRes = await query("SELECT count(*) AS total FROM labeled_results");
    const distRes = await query(
      "SELECT label_category, count(*) AS cnt FROM labeled_results GROUP BY label_category ORDER BY cnt DESC"
    );

    res.json({
      totalTorrents: parseInt(totalRes.rows[0].total, 10),
      totalLabeled: parseInt(labeledRes.rows[0].total, 10),
      categoryDistribution: distRes.rows.map((r) => ({
        category: r.label_category,
        count: parseInt(r.cnt, 10),
      })),
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error("GET /api/stats error:", msg);
    res.status(500).json({ error: msg });
  }
});

// SPA catch-all
app.get(/^(?!\/api)/, (_req, res) => {
  res.sendFile(path.join(distDir, "index.html"));
});

// --- Start ---
const HOST = process.env.HOST || "0.0.0.0";
const PORT = parseInt(process.env.PORT || "3002", 10);

startPython();

app.listen(PORT, HOST, () => {
  console.log(`Classifier web app running at http://${HOST}:${PORT}`);
});
