"""
Proof-of-Work solver using Obscura's V8 browser.

DeepSeek requires solving a DeepSeekHashV1 PoW challenge for each API call.
The WASM PoW solver is broken on ARM64 Mac, so we use Obscura's stealth browser
which runs on V8 and can load the WASM module correctly.

Usage:
    solver = ObscuraSolver(port=9222)
    answer = solver.solve(challenge)
"""

import json
import logging
import time
from dataclasses import dataclass
from typing import Optional

import httpx

logger = logging.getLogger(__name__)

WASM_URL = "https://fe-static.deepseek.com/chat/static/sha3_wasm_bg.7b9ca65ddd.wasm"

# The wasm-bindgen glue code that properly passes strings to the WASM module.
# This replicates DeepSeek's own worker JS exactly.
# Takes a single JSON arg: {challenge: "...", wasmB64: "..."}
WASM_SOLVE_JS = """
async ({challengeJson, wasmB64}) => {
    const ch = JSON.parse(challengeJson);

    const wasmBytes = Uint8Array.from(atob(wasmB64), c => c.charCodeAt(0));
    const {instance} = await WebAssembly.instantiate(wasmBytes, {});
    const n = instance.exports;

    let s = null;
    function o() {
        if (null === s || 0 === s.byteLength || s.buffer !== n.memory.buffer)
            s = new Uint8Array(n.memory.buffer);
        return s;
    }
    let c = null;
    function f() {
        if (null === c || true === c.buffer.detached ||
            void 0 === c.buffer.detached && c.buffer !== n.memory.buffer)
            c = new DataView(n.memory.buffer);
        return c;
    }
    const a = new TextEncoder();

    function u(str, malloc, realloc) {
        if (void 0 === realloc) {
            let bytes = a.encode(str);
            let ptr = malloc(bytes.length, 1) >>> 0;
            o().subarray(ptr, ptr + bytes.length).set(bytes);
            return ptr;
        }
        let len = str.length;
        let ptr = malloc(len, 1) >>> 0;
        let mem = o();
        let c = 0;
        for (; c < len; c++) {
            let ch = str.charCodeAt(c);
            if (ch > 127) break;
            mem[ptr + c] = ch;
        }
        if (c !== len) {
            if (c !== 0) str = str.slice(c);
            ptr = realloc(ptr, len, len = c + 3 * str.length, 1) >>> 0;
            let bytes = a.encode(str);
            o().subarray(ptr + c, ptr + c + bytes.length).set(bytes);
            c += bytes.length;
            ptr = realloc(ptr, len, c, 1) >>> 0;
        }
        return ptr;
    }

    const prefix = ch.salt + '_' + ch.expire_at + '_';
    let retptr = n.__wbindgen_add_to_stack_pointer(-16);
    let cp = u(ch.challenge, n.__wbindgen_export_0, n.__wbindgen_export_1);
    let pp = u(prefix, n.__wbindgen_export_0, n.__wbindgen_export_1);
    n.wasm_solve(retptr, cp, ch.challenge.length, pp, prefix.length, ch.difficulty);
    let status = f().getInt32(retptr + 0, true);
    let answer = Math.floor(f().getFloat64(retptr + 8, true));
    n.__wbindgen_add_to_stack_pointer(16);

    if (status === 0) return {error: 'PoW failed: status=0'};
    return {answer: answer};
}
"""


class ObscuraSolver:
    """PoW solver that uses Obscura's V8 browser to run DeepSeek's WASM."""

    def __init__(self, port: int = 9222, timeout: int = 60):
        self.port = port
        self.timeout = timeout
        self._playwright = None
        self._browser = None
        self._context = None
        self._page = None
        self._initialized = False

    def _ensure_init(self):
        """Lazy-initialize the Playwright connection to Obscura."""
        if self._initialized:
            return

        from playwright.sync_api import sync_playwright

        self._playwright = sync_playwright().start()
        self._browser = self._playwright.chromium.connect_over_cdp(
            f"http://127.0.0.1:{self.port}"
        )
        self._context = (
            self._browser.contexts[0]
            if self._browser.contexts
            else self._browser.new_context()
        )
        self._page = (
            self._context.pages[0]
            if self._context.pages
            else self._context.new_page()
        )

        # Navigate to about:blank for a clean context
        self._page.goto("about:blank", wait_until="commit", timeout=10000)
        time.sleep(0.3)

        # Pre-load WASM bytes in Python and cache as base64
        if not hasattr(self, "_wasm_b64"):
            import base64
            r = httpx.get(WASM_URL, timeout=30, follow_redirects=True)
            r.raise_for_status()
            self._wasm_b64 = base64.b64encode(r.content).decode()
            logger.info(f"Downloaded WASM: {len(r.content)} bytes")

        self._initialized = True
        logger.info("Obscura PoW solver initialized")

    def solve(self, challenge: dict) -> Optional[int]:
        """Solve a DeepSeekHashV1 PoW challenge.

        Args:
            challenge: dict with keys: algorithm, challenge, salt, difficulty,
                       signature, expire_at, target_path

        Returns:
            The nonce (answer) or None if solving failed.
        """
        self._ensure_init()

        challenge_json = json.dumps(challenge)
        arg = {"challengeJson": challenge_json, "wasmB64": self._wasm_b64}

        try:
            result = self._page.evaluate(WASM_SOLVE_JS, arg)
        except Exception as e:
            # Context destroyed — reinitialize and retry once
            logger.warning(f"PoW evaluate failed ({e}), reinitializing...")
            self._initialized = False
            self._ensure_init()
            try:
                result = self._page.evaluate(WASM_SOLVE_JS, arg)
            except Exception as e2:
                logger.error(f"PoW solve failed after retry: {e2}")
                return None

        if not result or result.get("error"):
            logger.error(f"PoW solve failed: {result}")
            return None

        answer = result.get("answer")
        if answer is None:
            logger.error("PoW solve returned no answer")
            return None

        return int(answer)

    def close(self):
        """Clean up browser resources."""
        if self._page:
            try:
                self._page.close()
            except Exception:
                pass
        if self._browser:
            try:
                self._browser.close()
            except Exception:
                pass
        if self._playwright:
            try:
                self._playwright.stop()
            except Exception:
                pass
        self._initialized = False
