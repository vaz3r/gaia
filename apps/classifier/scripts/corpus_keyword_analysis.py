#!/usr/bin/env python3
"""
Corpus-Wide 3.2M Torrent Keyword Cloud & Statistical Term Distinctiveness Engine.

Streams all 3,198,991 records from PostgreSQL using server-side cursors,
computes global and category-specific term frequencies, calculates cross-category
information entropy (for ubiquitous scene stop-words), and derives Monroe et al.
Dirichlet-prior Log-Odds Z-scores for every category.
"""

import sys
import os
import time
import re
import math
import json
from pathlib import Path
from collections import Counter

# Add src to sys.path
SRC_DIR = Path(__file__).parent.parent / "src"
sys.path.append(str(SRC_DIR))

import db

# Unicode-aware word token pattern: supports Latin, Cyrillic, CJK Ideographs, Kana, Hangul
TOKEN_RE = re.compile(r'(?u)[\w\u0400-\u04ff\u4e00-\u9fff\u3040-\u30ff\uac00-\ud7af]{2,35}')

ALL_CATEGORIES = [
    "Adult",
    "Anime",
    "Applications",
    "Audiobooks",
    "Books & Learning",
    "Documentaries",
    "Games",
    "Movies",
    "Music",
    "Television",
]


def run_corpus_analysis(sample_limit=None, output_json=None):
    print("=" * 85, flush=True)
    print("GAIA CORPUS KEYWORD CLOUD & DISTINCTIVENESS ENGINE (3.2M TORRENTS)", flush=True)
    print("=" * 85, flush=True)

    t0 = time.time()
    p = db.get_pool()
    conn = p.getconn()
    conn.set_session(readonly=True)

    with conn.cursor() as init_cur:
        init_cur.execute("SET statement_timeout = 0;")
        init_cur.execute("SET work_mem = '256MB';")

    cursor_name = f"corpus_cur_{int(time.time())}"
    cur = conn.cursor(name=cursor_name)
    cur.itersize = 50000

    query = """
        SELECT category, name, needs_review
        FROM torrents
    """
    if sample_limit:
        query += f" LIMIT {int(sample_limit)}"

    print(f"\n[1/4] Executing streaming query across torrents...", flush=True)
    cur.execute(query)

    # Statistical accumulators
    global_tf = Counter()
    global_df = Counter()
    cat_tf = {cat: Counter() for cat in ALL_CATEGORIES}
    cat_df = {cat: Counter() for cat in ALL_CATEGORIES}
    cat_doc_count = Counter()
    cat_token_count = Counter()

    queue_tf = Counter()
    queue_df = Counter()
    queue_doc_count = 0
    queue_token_count = 0

    total_docs = 0
    total_tokens = 0
    t_stream = time.time()

    print(f"[2/4] Streaming and tokenizing titles with Unicode regex...", flush=True)
    for row in cur:
        total_docs += 1
        cat = row[0]
        name = row[1] or ""
        needs_review = bool(row[2])

        # Clean delimiters
        cleaned = re.sub(r'[\._\-\+\[\]\(\)\{\}~!@#\$%\^&\*=\\/|<>,;:\'"?`]', ' ', name.lower())
        tokens = TOKEN_RE.findall(cleaned)
        token_set = set(tokens)

        n_tokens = len(tokens)
        total_tokens += n_tokens

        # Global counts
        global_tf.update(tokens)
        for w in token_set:
            global_df[w] += 1

        # Category-specific counts
        if cat in cat_tf:
            cat_doc_count[cat] += 1
            cat_token_count[cat] += n_tokens
            cat_tf[cat].update(tokens)
            for w in token_set:
                cat_df[cat][w] += 1

        # Review queue specific counts
        if needs_review:
            queue_doc_count += 1
            queue_token_count += n_tokens
            queue_tf.update(tokens)
            for w in token_set:
                queue_df[w] += 1

        if total_docs % 500000 == 0:
            rate = total_docs / max(time.time() - t_stream, 0.01)
            print(f"      Processed {total_docs:,} torrents ({rate:,.0f} docs/s) | Unique vocab: {len(global_tf):,}...", flush=True)

    cur.close()
    conn.rollback()
    p.putconn(conn)

    duration = time.time() - t_stream
    print(f"\n      ✓ Completed streaming in {duration:.1f}s ({total_docs / max(duration, 0.01):,.0f} docs/s)")
    print(f"      Total documents: {total_docs:,}")
    print(f"      Total tokens: {total_tokens:,}")
    print(f"      Unique vocabulary: {len(global_tf):,}")
    print(f"      Review queue documents: {queue_doc_count:,} ({queue_token_count:,} tokens)")

    # 3. Ubiquitous Scene Stop-Word Discovery (Entropy Analysis)
    # 3. Ubiquitous Scene Stop-Words Discovery
    print(f"\n[3/4] Identifying ubiquitous noise and cross-category stop-words...", flush=True)
    min_noise_df = max(50, int(total_docs * 0.0005))
    noise_candidates = [w for w, count in global_df.items() if count >= min_noise_df]
    print(f"      Noise candidates with DF >= {min_noise_df}: {len(noise_candidates):,} terms")

    stop_word_scores = []
    log2_num_cats = math.log2(len(ALL_CATEGORIES))

    for w in noise_candidates:
        df_w = global_df[w]
        probs = []
        for cat in ALL_CATEGORIES:
            c_df = cat_df[cat].get(w, 0)
            if c_df > 0 and cat_doc_count[cat] > 0:
                p_c = c_df / cat_doc_count[cat]
                probs.append(p_c)
            else:
                probs.append(0.0)

        p_sum = sum(probs)
        if p_sum > 0:
            norm_p = [p / p_sum for p in probs]
            entropy = -sum(p * math.log2(p) for p in norm_p if p > 0)
        else:
            entropy = 0.0

        # High entropy means the term appears proportionally across many categories
        norm_entropy = entropy / log2_num_cats
        noise_score = norm_entropy * math.log10(max(df_w, 1))
        # Only true cross-category terms (normalized entropy >= 0.60)
        if norm_entropy >= 0.60:
            stop_word_scores.append((w, df_w, entropy, noise_score))

    stop_word_scores.sort(key=lambda x: x[3], reverse=True)
    top_noise_words = set(x[0] for x in stop_word_scores[:200])
    print(f"      Identified {len(top_noise_words)} ubiquitous cross-category stop-words")

    print("\n" + "-" * 75)
    print(f"{'Rank':<5} | {'Term':<25} | {'Doc Freq':<12} | {'Entropy':<10} | {'Noise Score':<10}")
    print("-" * 75)
    for i, (w, df_w, ent, ns) in enumerate(stop_word_scores[:25], 1):
        print(f"{i:<5} | {w:<25} | {df_w:<12,} | {ent:<10.3f} | {ns:<10.2f}")

    # 4. Category-Specific Distinctiveness (Log-Odds with Dirichlet Prior)
    print(f"\n[4/4] Computing Monroe Dirichlet Log-Odds Category Keyword Clouds...", flush=True)
    alpha_0 = 5000.0
    total_bg_tokens = max(total_tokens, 1)

    min_cat_freq = max(10, int(total_docs * 0.0001))
    category_clouds = {}

    for cat in ALL_CATEGORIES:
        n_c = cat_token_count[cat]
        n_not_c = total_tokens - n_c
        c_tf = cat_tf[cat]

        scores = []
        for w, f_c in c_tf.items():
            if f_c < min_cat_freq:
                continue
            if w in top_noise_words:
                continue

            f_not_c = global_tf[w] - f_c
            a_w = (global_tf[w] / total_bg_tokens) * alpha_0

            num_c = (f_c + a_w) / max(n_c + alpha_0 - (f_c + a_w), 1.0)
            num_not_c = (f_not_c + a_w) / max(n_not_c + alpha_0 - (f_not_c + a_w), 1.0)

            delta = math.log(max(num_c, 1e-12)) - math.log(max(num_not_c, 1e-12))
            sigma2 = (1.0 / (f_c + a_w)) + (1.0 / (f_not_c + a_w))
            z_score = delta / math.sqrt(sigma2)

            scores.append((w, f_c, global_tf[w], z_score))

        scores.sort(key=lambda x: x[3], reverse=True)
        category_clouds[cat] = scores[:100]

    # Display Top 15 keywords for each category
    print("\n" + "=" * 85)
    print("EMPIRICAL CATEGORY KEYWORD CLOUDS (TOP DISTINCTIVE TERMS)")
    print("=" * 85)

    for cat in ALL_CATEGORIES:
        top_terms = category_clouds[cat][:15]
        print(f"\n📂 [{cat.upper()}] ({cat_doc_count[cat]:,} torrents):")
        terms_str = ", ".join(f"{w} (z={z:.1f})" for w, fc, gt, z in top_terms)
        print(f"   {terms_str}")

    # 5. Review Queue Distinctive Term Fingerprint
    print("\n" + "=" * 85)
    print("REVIEW QUEUE SEMANTIC FINGERPRINT (TOP OVER-INDEXED TERMS)")
    print("=" * 85)

    queue_scores = []
    n_q = queue_token_count
    for w, f_q in queue_tf.items():
        if f_q < 5:
            continue
        if w in top_noise_words:
            continue
        f_not_q = global_tf[w] - f_q
        a_w = (global_tf[w] / total_bg_tokens) * alpha_0

        num_q = (f_q + a_w) / max(n_q + alpha_0 - (f_q + a_w), 1.0)
        num_not_q = (f_not_q + a_w) / max(total_tokens - n_q + alpha_0 - (f_not_q + a_w), 1.0)

        delta = math.log(max(num_q, 1e-12)) - math.log(max(num_not_q, 1e-12))
        sigma2 = (1.0 / (f_q + a_w)) + (1.0 / (f_not_q + a_w))
        z = delta / math.sqrt(sigma2)
        queue_scores.append((w, f_q, z))

    queue_scores.sort(key=lambda x: x[2], reverse=True)
    top_queue_terms = queue_scores[:30]
    print(f"\n🔍 [REVIEW QUEUE OVER-INDEXED TERMS] ({queue_doc_count:,} items):")
    for w, fq, z in top_queue_terms:
        best_cat = "Unknown"
        best_cat_tf = 0
        for c in ALL_CATEGORIES:
            if cat_tf[c].get(w, 0) > best_cat_tf:
                best_cat_tf = cat_tf[c].get(w, 0)
                best_cat = c
        print(f"   * {w:<25} | in review: {fq:<6} | z-score: {z:<6.1f} | matches: {best_cat}")

    # Compile JSON report
    report = {
        "metadata": {
            "total_torrents": total_docs,
            "total_tokens": total_tokens,
            "vocab_size": len(global_tf),
            "review_queue_torrents": queue_doc_count,
            "analyzed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "duration_seconds": round(time.time() - t0, 1),
        },
        "top_noise_words": [
            {"term": w, "doc_freq": df_w, "entropy": round(ent, 3), "noise_score": round(ns, 2)}
            for w, df_w, ent, ns in stop_word_scores[:150]
        ],
        "category_clouds": {
            cat: [
                {"term": w, "cat_tf": fc, "global_tf": gt, "z_score": round(z, 2)}
                for w, fc, gt, z in category_clouds[cat]
            ]
            for cat in ALL_CATEGORIES
        },
        "review_queue_fingerprint": [
            {"term": w, "queue_tf": fq, "z_score": round(z, 2)}
            for w, fq, z in queue_scores[:100]
        ]
    }

    out_path = output_json or "/tmp/corpus_keyword_report.json"
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=2)
    print(f"\n✓ Analysis report saved to {out_path} ({os.path.getsize(out_path) / 1024:.1f} KB)", flush=True)

    return report


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="Corpus Keyword Cloud Engine")
    parser.add_argument("--limit", type=int, default=None, help="Limit number of rows to analyze (for testing)")
    parser.add_argument("--output", type=str, default="/tmp/corpus_keyword_report.json", help="Output JSON path")
    args = parser.parse_args()

    run_corpus_analysis(sample_limit=args.limit, output_json=args.output)
