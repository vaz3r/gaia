# M365 Copilot Client

A Node.js/Express API that reverse-engineers the Microsoft 365 Copilot chat WebSocket protocol, exposing a simple `POST /chat` endpoint for sending messages and receiving responses with multi-turn conversation support.

## Features

- **Automatic token refresh** — PKCE browser flow with MSAL, tokens persist in `.token.json` and auto-refresh
- **Multi-turn conversations** — Maintains conversation context via `conversationId`
- **Fresh WebSocket per request** — Matches real browser behavior (each message opens a new connection)
- **Full protocol support** — SignalR wire format with proper handshake, ping, metrics, and cleanup
- **Conversation cleanup** — Auto-delete server-created conversations, bulk cleanup endpoints
- **Server vs user tracking** — Only deletes conversations created by the API, not your browser conversations

## Quick Start

### 1. Install dependencies

```bash
cd m365-copilot-client
npm install
```

### 2. Start the server

```bash
node server.js
```

### 3. Full curl flow: Auth → Chat → Multi-turn → Cleanup

```bash
# ─── STEP 1: Start server (in one terminal) ───
node server.js

# ─── STEP 2: Get auth URL (in another terminal) ───
curl -s -X POST http://localhost:3000/auth/login | python3 -m json.tool
# Returns:
# {
#   "authUrl": "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id=...",
#   "instructions": [...]
# }

# ─── STEP 3: Open the authUrl in your browser ───
# Sign in with your Microsoft account
# After redirect, COPY the full URL from address bar
# It looks like: https://login.microsoftonline.com/common/oauth2/nativeclient?code=AAAA...

# ─── STEP 4: Complete authentication ───
curl -s -X POST http://localhost:3000/auth/complete \
  -H "Content-Type: application/json" \
  -d '{"redirectUrl": "PASTE_YOUR_REDIRECT_URL_HERE"}'
# Returns:
# {
#   "success": true,
#   "message": "Authentication complete!",
#   "expiresIn": 4639
# }

# ─── STEP 5: Verify auth status ───
curl -s http://localhost:3000/auth/status | python3 -m json.tool
# Returns:
# {
#   "authenticated": true,
#   "hasRefreshToken": true,
#   "expiresAt": "2026-09-26T16:30:00.000Z",
#   "expiresInMinutes": 77
# }

# ─── STEP 6: First chat message ───
RESPONSE=$(curl -s -m 90 -X POST http://localhost:3000/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "What is 2+2?"}')

echo "$RESPONSE" | python3 -m json.tool
# Returns:
# {
#   "text": "2 + 2 = **4**.",
#   "messageId": "...",
#   "conversationId": "ea193eb6-a094-4cca-96ce-0362ebc71f0e",
#   "references": [...],
#   "sourceAttributions": [...],
#   "suggestedResponses": [...]
# }

# Extract conversationId for follow-up messages
CONV_ID=$(echo "$RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['conversationId'])")
echo "Conversation ID: $CONV_ID"

# ─── STEP 7: Multi-turn follow-up ───
curl -s -m 90 -X POST http://localhost:3000/chat \
  -H "Content-Type: application/json" \
  -d "{\"message\": \"Now multiply by 3\", \"conversationId\": \"$CONV_ID\"}" \
  | python3 -m json.tool
# Returns:
# {
#   "text": "4 × 3 = **12**.",
#   "conversationId": "ea193eb6-a094-4cca-96ce-0362ebc71f0e",
#   ...
# }

# ─── STEP 8: Another follow-up (same conversation) ───
curl -s -m 90 -X POST http://localhost:3000/chat \
  -H "Content-Type: application/json" \
  -d "{\"message\": \"Now add 100\", \"conversationId\": \"$CONV_ID\"}" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['text'])"
# Returns: 12 + 100 = **112**.

# ─── STEP 9: Chat with auto-delete (for bulk processing) ───
curl -s -m 90 -X POST http://localhost:3000/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "This conversation will be auto-deleted", "autoDelete": true}'
# Returns:
# {
#   "text": "...",
#   "conversationId": "...",
#   "deleted": true
# }

# ─── STEP 10: List server-tracked conversations ───
curl -s http://localhost:3000/conversations | python3 -m json.tool
# Returns:
# {
#   "count": 1,
#   "conversations": [
#     {
#       "conversationId": "ea193eb6-a094-4cca-96ce-0362ebc71f0e",
#       "createdAt": "2026-09-26T18:00:00.000Z",
#       "firstMessage": "What is 2+2?"
#     }
#   ]
# }

# ─── STEP 11: Delete specific conversations ───
curl -s -X POST http://localhost:3000/conversations/delete \
  -H "Content-Type: application/json" \
  -d "{\"conversationIds\": [\"$CONV_ID\"]}"
# Returns:
# {
#   "deleted": 1,
#   "ids": ["ea193eb6-a094-4cca-96ce-0362ebc71f0e"]
# }

# ─── STEP 12: Cleanup all server-tracked conversations ───
curl -s -X POST http://localhost:3000/conversations/cleanup
# Returns:
# {
#   "deleted": 0,
#   "message": "No server-tracked conversations to clean up"
# }

# ─── STEP 13: Logout (optional) ───
curl -s -X POST http://localhost:3000/auth/logout | python3 -m json.tool
# Returns: { "ok": true }
```

## API Endpoints

### Authentication

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/auth/login` | Returns the browser auth URL for PKCE flow |
| `POST` | `/auth/complete` | Exchanges redirect URL for tokens |
| `GET` | `/auth/status` | Check authentication status and token expiry |
| `POST` | `/auth/logout` | Clear stored tokens |

### Chat

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/chat` | Send a message to Copilot |

#### POST /chat

**Request body:**

```json
{
  "message": "What is 2+2?",
  "conversationId": "optional-conversation-id",
  "token": "optional-manual-token-override",
  "timeout": 60000,
  "autoDelete": false
}
```

| Field | Type | Description |
|-------|------|-------------|
| `message` | string | **Required.** The message to send |
| `conversationId` | string | Optional. For multi-turn conversations |
| `token` | string | Optional. Manual token override (bypasses auto-refresh) |
| `timeout` | number | Optional. Response timeout in ms (default: 60000) |
| `autoDelete` | boolean | Optional. Auto-delete conversation after response |

**Response:**

```json
{
  "text": "2 + 2 = **4**.",
  "messageId": "...",
  "conversationId": "ea193eb6-a094-4cca-96ce-0362ebc71f0e",
  "references": [],
  "sourceAttributions": [],
  "suggestedResponses": [],
  "deleted": false
}
```

### Conversation Management

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/conversations` | List server-tracked conversations |
| `POST` | `/conversations/delete` | Delete specific conversations |
| `POST` | `/conversations/cleanup` | Delete all server-tracked conversations |

#### POST /conversations/delete

**Request body:**

```json
{
  "conversationIds": ["id1", "id2", "id3"]
}
```

Or single conversation:

```json
{
  "conversationId": "single-id"
}
```

**Response:**

```json
{
  "deleted": 3,
  "ids": ["id1", "id2", "id3"]
}
```

#### POST /conversations/cleanup

Deletes all conversations tracked by the server (created via `/chat` endpoint).

**Request body:** None

**Response:**

```json
{
  "deleted": 5,
  "ids": ["id1", "id2", "id3", "id4", "id5"]
}
```

## Bulk Processing Example

For bulk data processing, use auto-delete to keep your conversation history clean:

```bash
#!/bin/bash
# bulk_process.sh - Process data and auto-clean conversations

# Array of items to process
ITEMS=("item1" "item2" "item3" "item4" "item5")

for item in "${ITEMS[@]}"; do
  # Chat with auto-delete=true cleans up after each request
  RESPONSE=$(curl -s -m 90 -X POST http://localhost:3000/chat \
    -H "Content-Type: application/json" \
    -d "{\"message\": \"Process this: $item\", \"autoDelete\": true}")
  
  echo "Processed: $item"
  echo "Response: $(echo "$RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin).get('text','?'))")"
  echo "---"
done

# Final cleanup to ensure nothing is left
curl -s -X POST http://localhost:3000/conversations/cleanup
echo "All server conversations cleaned up!"
```

## How It Works

### Authentication Flow

1. **PKCE Browser Flow** — Uses Microsoft's first-party Office web app (`c0ab8ce9-e9a0-42e7-b064-33d422df41f1`)
2. **Redirect URI** — `https://login.microsoftonline.com/common/oauth2/nativeclient` (the only registered redirect for this app)
3. **Token Storage** — Tokens persist in `.token.json` with automatic refresh before expiry
4. **Scopes** — `https://substrate.office.com/sydney/.default` (full read/write/delete access)

### Conversation Tracking

The server tracks conversations it creates separately from conversations you create in the browser:

- **Server-created**: Conversations created via `POST /chat` without `conversationId` — these are tracked and can be auto-deleted
- **User-created**: Conversations created in the browser or passed with `conversationId` — these are **never** tracked or deleted by the server

### WebSocket Protocol

The Copilot chat uses **SignalR over WebSocket** with the following wire format:

- Messages delimited by `\x1e` (record separator)
- JSON payloads with `type` field:
  - `1` — Invocation (client→server commands, server→client updates)
  - `2` — StreamItem (full turn result)
  - `3` — Completion (invocation finished)
  - `4` — StreamInvocation (client→server chat request)
  - `6` — Ping
  - `7` — Close

### Message Flow

```
Client                          Server
  |--- { protocol: "json" } ---->|
  |<-- { type: 0 } -------------|  (handshake)
  |                              |
  |--- { type: 6 } ------------>|  (ping every 15s)
  |<-- { type: 6 } -------------|
  |                              |
  |--- { type: 1, target:       |
  |     "send", activityType:   |
  |     "TypingStarted" } ----->|
  |                              |
  |--- { type: 4, target:       |
  |     "chat", args: [payload]}|--  (chat request)
  |                              |
  |<-- { type: 1, target:       |
  |     "update", messages } ---|  (streaming updates)
  |                              |
  |<-- { type: 2, item:        -|  (full result)
  |     { messages } }          |
  |                              |
  |<-- { type: 3 } -------------|  (completion)
  |                              |
  |--- { type: 1, target:       |
  |     "Metrics" } ----------->|  (client metrics)
```

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | Server port |

### Constants (in `server.js`)

| Constant | Value | Description |
|----------|-------|-------------|
| `USER_ID` | `68ce272e-092a-460f-b4a3-20ad9501f523` | Fixed user OID |
| `TENANT_ID` | `544b0e7e-1de4-43f2-8d46-d9acd0f15d60` | Tenant ID |
| `BROWSER_SESSION_ID` | `dc106e5f-28de-0732-3ca5-a71c242c4c6a` | Constant session ID across connections |

## File Structure

```
m365-copilot-client/
├── server.js          # Express API + WebSocket protocol + conversation management
├── auth.js            # MSAL PKCE browser auth + token refresh
├── client.js          # CLI REPL client (reference implementation)
├── package.json       # Dependencies (ws, express, uuid)
├── .gitignore         # Excludes .token.json and .conversations.json
├── .token.json        # Stored tokens (auto-generated, git-ignored)
├── .conversations.json # Persisted conversation tracking (git-ignored)
└── README.md          # This file
```

## Technical Details

### Why Fresh WebSocket Per Request?

The M365 Copilot server does not process new messages on reused WebSocket connections. Each `POST /chat` must open a new `wss://substrate.office.com/m365Copilot/Chathub/...` connection, matching how the real browser behaves.

### Why PKCE Instead of Device Code Flow?

The app `c0ab8ce9-e9a0-42e7-b064-33d422df41f1` is Microsoft's first-party Office web app. Azure AD requires preauthorization for first-party apps using device code flow (error `AADSTS65002`). PKCE browser flow works because it uses the app's registered redirect URI (`https://login.microsoftonline.com/common/oauth2/nativeclient`).

### Token Lifetime

- **Access token**: ~77 minutes (auto-refreshes at 5 minutes before expiry)
- **Refresh token**: Valid for ~90 days (refreshes on each use)
- **Token storage**: `.token.json` in project root

### Conversation Context

- `conversationId` is constant across connections (maintains context server-side)
- `chatSessionId` changes per connection (new UUID per request)
- `invocationId` is always `"0"` for every message

### Conversation Cleanup

- **Server tracking**: Only conversations created by the API are tracked (new conversations without `conversationId`)
- **Auto-delete**: Set `autoDelete: true` in `/chat` request to delete immediately after response
- **Bulk cleanup**: Use `POST /conversations/cleanup` to delete all tracked conversations
- **User conversations**: Conversations created in the browser are **never** touched by cleanup endpoints
- **Persistence**: Conversation tracking persists to disk (`.conversations.json`) across server restarts

## Troubleshooting

### "Not authenticated" error

```bash
# Check auth status
curl http://localhost:3000/auth/status

# Re-authenticate if needed
curl -X POST http://localhost:3000/auth/login
```

### Token expired

The server auto-refreshes tokens, but if the refresh token expires (~90 days):

```bash
curl -X POST http://localhost:3000/auth/logout
curl -X POST http://localhost:3000/auth/login
# Follow the auth flow again
```

### WebSocket connection fails

Ensure the server can reach `substrate.office.com` on port 443. Check firewall settings if running in a restricted environment.

### Delete conversation fails

The delete endpoint uses the same OAuth token as chat. Ensure:
1. Token is valid (check `/auth/status`)
2. Token has the right scopes (`https://substrate.office.com/sydney/.default`)
3. The conversation ID exists and belongs to your account

## Acknowledgments

This project reverse-engineers the M365 Copilot chat protocol from browser traffic analysis. It is not affiliated with or endorsed by Microsoft.
