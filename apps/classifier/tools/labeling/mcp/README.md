# Gaia Torrent Classifier MCP Server for OpenCode

Run automated, high-precision torrent classification directly from **OpenCode** using **Minimax Mimo 2.5 Pro** over Tailscale at $0.00 cost.

## What This MCP Server Does
1. Connects to PostgreSQL directly from your Mac over Tailscale.
2. Exposes three native MCP tools to OpenCode:
   - `get_labeling_instructions()`: Returns the official 10-category taxonomy + rules.
   - `get_review_queue_batch(limit=50, after_infohash=...)`: Fetches batches of torrents from the Review Queue using sub-30ms keyset queries.
   - `record_classifications(results)`: Writes ground truth to `labeled_results` and **instantly clears `needs_review = false`**, draining the Review Queue in real-time.
   - `get_queue_statistics()`: Returns live counts of the Review Queue and ground truth labels.

---

## 1-Minute Setup in OpenCode

### 1. Register the MCP Server in OpenCode
Add the following to your OpenCode configuration (or `opencode.json`):

```json
{
  "mcpServers": {
    "gaia-classifier": {
      "command": "/absolute/path/to/gaia/apps/classifier/tools/labeling/mcp/run.sh"
    }
  }
}
```
*(Replace `/absolute/path/to/gaia` with the actual path to your gaia repository on your Mac).*

### 2. Tailscale Connection
Ensure Tailscale is active on your Mac so the script can reach `workspace-production` or `100.87.194.112`.

---

## How to Prompt OpenCode

In an OpenCode conversation with **Mimo 2.5 Pro**, simply paste this prompt:

> *"Please use the `gaia-classifier` MCP tools to fetch a batch of torrents from the review queue (`get_review_queue_batch`), classify each torrent into its correct category according to `get_labeling_instructions`, and save the results using `record_classifications`. Continue loop until 1,000 torrents are labeled."*

OpenCode will call the tools in a loop, streaming classifications and draining your review queue directly into PostgreSQL!
