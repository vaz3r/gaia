const https = require("https");
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const { URL } = require("url");

// M365 Copilot app (Microsoft's first-party Office web app)
const CLIENT_ID = "c0ab8ce9-e9a0-42e7-b064-33d422df41f1";
const AUTHORITY = "https://login.microsoftonline.com/common";
// This is the only redirect URI registered for this first-party app
const REDIRECT_URI = "https://login.microsoftonline.com/common/oauth2/nativeclient";

// Resource-specific scopes for Copilot API (read + write + delete)
const SCOPES = [
  "https://substrate.office.com/sydney/.default",
  "offline_access",
];

const TOKEN_FILE = path.join(__dirname, ".token.json");

function base64url(buf) {
  return buf.toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

class M365Auth {
  constructor() {
    this.accessToken = null;
    this.refreshToken = null;
    this.expiresAt = null;
    this._codeVerifier = null;
    this._loadToken();
  }

  // --- Token persistence ---
  _loadToken() {
    try {
      if (fs.existsSync(TOKEN_FILE)) {
        const data = JSON.parse(fs.readFileSync(TOKEN_FILE, "utf8"));
        this.accessToken = data.accessToken;
        this.refreshToken = data.refreshToken;
        this.expiresAt = data.expiresAt;
        if (this.isTokenValid()) {
          console.log("[auth] Loaded cached token, expires in", Math.round((this.expiresAt - Date.now()) / 60000), "min");
        } else if (this.refreshToken) {
          console.log("[auth] Token expired, will refresh...");
        }
      }
    } catch { /* ignore */ }
  }

  _saveToken() {
    try {
      fs.writeFileSync(TOKEN_FILE, JSON.stringify({
        accessToken: this.accessToken,
        refreshToken: this.refreshToken,
        expiresAt: this.expiresAt,
      }, null, 2));
    } catch (e) {
      console.error("[auth] Failed to save token:", e.message);
    }
  }

  // --- HTTP helper ---
  _post(url, body) {
    return new Promise((resolve, reject) => {
      const urlObj = new URL(url);
      const postData = new URLSearchParams(body).toString();
      const req = https.request({
        hostname: urlObj.hostname,
        path: urlObj.pathname,
        method: "POST",
        headers: {
          "Content-Type": "application/x-www-form-urlencoded",
          "Content-Length": Buffer.byteLength(postData),
        },
      }, (res) => {
        let data = "";
        res.on("data", (c) => data += c);
        res.on("end", () => {
          try {
            const json = JSON.parse(data);
            if (res.statusCode >= 400) {
              reject(new Error(json.error_description || json.error || `HTTP ${res.statusCode}`));
            } else {
              resolve(json);
            }
          } catch (e) {
            reject(new Error(`Parse error: ${data.substring(0, 200)}`));
          }
        });
      });
      req.on("error", reject);
      req.write(postData);
      req.end();
    });
  }

  // --- Browser Auth Flow (nativeclient redirect) ---
  async getAuthUrl() {
    // Generate PKCE code_verifier and code_challenge
    const codeVerifier = base64url(crypto.randomBytes(32));
    const codeChallenge = base64url(crypto.createHash("sha256").update(codeVerifier).digest());
    this._codeVerifier = codeVerifier;

    const params = new URLSearchParams({
      client_id: CLIENT_ID,
      response_type: "code",
      redirect_uri: REDIRECT_URI,
      scope: SCOPES.join(" "),
      code_challenge: codeChallenge,
      code_challenge_method: "S256",
      response_mode: "query",
    });

    return `${AUTHORITY}/oauth2/v2.0/authorize?${params.toString()}`;
  }

  async completeAuth(redirectUrl) {
    if (!this._codeVerifier) {
      throw new Error("No auth flow in progress. Call /auth/login first.");
    }

    // Extract auth code from URL
    const url = new URL(redirectUrl);
    const authCode = url.searchParams.get("code");
    const error = url.searchParams.get("error");

    if (error) {
      throw new Error(url.searchParams.get("error_description") || error);
    }

    if (!authCode) {
      throw new Error("No authorization code found in URL");
    }

    // Exchange auth code for tokens
    const tokens = await this._exchangeCodeForTokens(authCode);

    this.accessToken = tokens.access_token;
    this.refreshToken = tokens.refresh_token;
    this.expiresAt = Date.now() + (tokens.expires_in * 1000);
    this._codeVerifier = null;

    this._saveToken();
    console.log("[auth] Token acquired! Expires in", Math.round(tokens.expires_in / 60000), "min");
    return { accessToken: this.accessToken, expiresIn: tokens.expires_in };
  }

  async _exchangeCodeForTokens(authCode) {
    return this._post(`${AUTHORITY}/oauth2/v2.0/token`, {
      client_id: CLIENT_ID,
      code: authCode,
      redirect_uri: REDIRECT_URI,
      grant_type: "authorization_code",
      code_verifier: this._codeVerifier,
      scope: SCOPES.join(" "),
    });
  }

  // --- Token Refresh ---
  async refreshAccessToken() {
    if (!this.refreshToken) throw new Error("No refresh token. Run /auth/login first.");

    try {
      const result = await this._post(`${AUTHORITY}/oauth2/v2.0/token`, {
        grant_type: "refresh_token",
        client_id: CLIENT_ID,
        scope: SCOPES.join(" "),
        refresh_token: this.refreshToken,
      });

      this.accessToken = result.access_token;
      if (result.refresh_token) this.refreshToken = result.refresh_token;
      this.expiresAt = Date.now() + (result.expires_in * 1000);

      this._saveToken();
      console.log("[auth] Token refreshed! Expires in", Math.round(result.expires_in / 60000), "min");
      return this.accessToken;
    } catch (err) {
      console.error("[auth] Refresh failed:", err.message);
      throw err;
    }
  }

  // --- Get valid token (auto-refresh if needed) ---
  async getValidToken() {
    if (!this.accessToken && !this.refreshToken) {
      throw new Error("Not authenticated. Call /auth/login first.");
    }

    // Refresh if expired or expiring within 5 minutes
    if (!this.isTokenValid() || (this.expiresAt - Date.now()) < 5 * 60 * 1000) {
      console.log("[auth] Token expiring soon, refreshing...");
      await this.refreshAccessToken();
    }

    return this.accessToken;
  }

  isTokenValid() {
    return this.accessToken && this.expiresAt && Date.now() < this.expiresAt;
  }

  isAuthenticated() {
    return this.isTokenValid() || !!this.refreshToken;
  }

  getAuthInfo() {
    return {
      authenticated: this.isTokenValid(),
      hasRefreshToken: !!this.refreshToken,
      expiresAt: this.expiresAt ? new Date(this.expiresAt).toISOString() : null,
      expiresInMinutes: this.expiresAt ? Math.round((this.expiresAt - Date.now()) / 60000) : 0,
    };
  }

  logout() {
    this.accessToken = null;
    this.refreshToken = null;
    this.expiresAt = null;
    try { fs.unlinkSync(TOKEN_FILE); } catch { /* ignore */ }
    console.log("[auth] Logged out");
  }
}

module.exports = M365Auth;
