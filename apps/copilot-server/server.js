const WebSocket = require("ws");
const express = require("express");
const crypto = require("crypto");
const https = require("https");
const fs = require("fs");
const path = require("path");
const M365Auth = require("./auth");

const auth = new M365Auth();

// --- Constants ---
const SR_INVOCATION = 1;
const SR_INVOCATION_RESULT = 2;
const SR_COMPLETION = 3;
const SR_STREAM_INVOCATION = 4;
const SR_PING = 6;
const SR_CLOSE = 7;
const RECORD_SEPARATOR = "\x1e";

const USER_ID = "68ce272e-092a-460f-b4a3-20ad9501f523";
const TENANT_ID = "544b0e7e-1de4-43f2-8d46-d9acd0f15d60";
const BROWSER_SESSION_ID = "dc106e5f-28de-0732-3ca5-a71c242c4c6a";

// --- Conversation Persistence ---
const CONVERSATIONS_FILE = path.join(__dirname, ".conversations.json");

function loadConversations() {
  try {
    if (fs.existsSync(CONVERSATIONS_FILE)) {
      const data = JSON.parse(fs.readFileSync(CONVERSATIONS_FILE, "utf8"));
      const map = new Map();
      for (const [key, value] of Object.entries(data)) {
        map.set(key, value);
      }
      console.log(`[store] Loaded ${map.size} conversations from disk`);
      return map;
    }
  } catch (err) {
    console.error("[store] Failed to load conversations:", err.message);
  }
  return new Map();
}

function saveConversations() {
  try {
    const obj = Object.fromEntries(serverConversations);
    fs.writeFileSync(CONVERSATIONS_FILE, JSON.stringify(obj, null, 2));
  } catch (err) {
    console.error("[store] Failed to save conversations:", err.message);
  }
}

// --- Conversation Tracking ---
// Tracks conversations created by the server (not by the user in browser)
let serverConversations = loadConversations();

function trackConversation(conversationId, firstMessage) {
  if (conversationId && !serverConversations.has(conversationId)) {
    serverConversations.set(conversationId, {
      createdAt: new Date().toISOString(),
      firstMessage: firstMessage?.substring(0, 100),
    });
    saveConversations();
    console.log(`[track] Conversation created: ${conversationId}`);
  }
}

function untrackConversation(conversationId) {
  serverConversations.delete(conversationId);
  saveConversations();
}

// --- Delete Conversation via Substrate API ---
async function deleteConversations(conversationIds) {
  if (!conversationIds || conversationIds.length === 0) return { deleted: 0 };

  const token = await auth.getValidToken();
  const traceId = crypto.randomUUID();

  return new Promise((resolve, reject) => {
    const postData = JSON.stringify({
      conversationIdsToDelete: conversationIds,
      source: "officeweb",
      traceId,
    });

    const req = https.request({
      hostname: "substrate.office.com",
      path: "/m365Copilot/DeleteConversation",
      method: "POST",
      headers: {
        "Authorization": `Bearer ${token}`,
        "Content-Type": "application/json",
        "Content-Length": Buffer.byteLength(postData),
        "x-anchormailbox": `Oid:${USER_ID}@${TENANT_ID}`,
        "x-routingparameter-sessionkey": USER_ID,
        "x-scenario": "OfficeWebIncludedCopilot",
        "x-clientrequestid": crypto.randomUUID(),
      },
    }, (res) => {
      let data = "";
      res.on("data", (c) => data += c);
      res.on("end", () => {
        if (res.statusCode >= 200 && res.statusCode < 300) {
          console.log(`[delete] Deleted ${conversationIds.length} conversation(s)`);
          conversationIds.forEach(id => untrackConversation(id));
          resolve({ deleted: conversationIds.length, ids: conversationIds });
        } else {
          console.error(`[delete] Failed: ${res.statusCode} ${data.substring(0, 200)}`);
          reject(new Error(`Delete failed: ${res.statusCode}`));
        }
      });
    });

    req.on("error", reject);
    req.write(postData);
    req.end();
  });
}

const CHROME_HEADERS = {
  "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36",
  "Accept-Language": "en-US,en;q=0.9,fr;q=0.8,pt;q=0.7,ar;q=0.6",
  "Cache-Control": "no-cache",
  Pragma: "no-cache",
  Origin: "https://m365.cloud.microsoft",
};

const VARIANTS = [
  "EnableMcpServerWidgets","feature.EnableMcpServerWidgets",
  "feature.EnableImageGenInsufficientTokensThrottled",
  "feature.EnableImageGenSystemCapacityThrottled",
  "feature.EnableLuForChatCIQ","feature.enableChatCIQPlugin",
  "EnableRequestPlugins","feature.EnableSensitivityLabels",
  "EnableUnsupportedUrlDetector","feature.IsCustomEngineCopilotEnabled",
  "feature.bizchatfluxv3","feature.enablechatpages",
  "feature.enableCodeCanvas","feature.turnOnDARecommendation",
  "feature.IsStreamingModeInChatRequestEnabled",
  "IncludeSourceAttributionsConcise","SkipPublishEmptyMessage",
  "feature.EnableDeduplicatingSourceAttributions",
  "feature.IsCitationsReferencesOutputEnabled",
  "feature.enableDeltaStreamingForReferences",
  "feature.enableIncludeReferencesInDeltaResponse",
  "feature.enablereferencesforagents",
  "feature.EnableCodeInterpreterConversion",
  "agt_module_attr_enableReferencesForCodeInterpreter",
  "agt_module_enableCodeInterpreterHallucinatedUrlFilter",
  "agt_module_attr_enableCodeInterpreterFilePreviewReference",
  "cdxcipreviewmsg","Enable3PActionProgressMessages",
  "feature.enableClientWebRtc",
  "feature.EnableMeetingRecapOfSeriesMeetingWithCiq",
  "feature.EnableReferencesListCompleteSignal",
  "StorageMessageSplitDisabled","SingletonEnvOn",
  "cdxenablefccinmainline",
  "agt_bizchat_enableSearchResultProgressMessages",
  "EnableComposeWidget","feature.EnableResearcherTodoListObserver",
  "feature.EnableResearcherTodoObserverSlim","feature.EnableResearchSteering",
  "feature.EnableResearcherTodoSummarizerPacing",
  "agt_researcheragent_enableMemoryRead",
  "feature.EnableShoppingCashback","feature.cwcallowedos",
  "feature.EnableMergingPureDeltas","feature.disabledisallowedmsgs",
  "feature.enableCitationsForSynthesisData",
  "feature.EnableConversationShareApis",
  "feature.EnableConversationShareApisForMsa",
  "feature.EnableGoodbyeDrainGate",
  "feature.enableGenerateGraphicArtOptionsSet","cdximagen",
  "feature.EnableUpdatedUXForConfirmationDialog",
  "feature.EnableContentApiandDocTypeHtmlInRichAnswers",
  "cdxgrounding_api_v2_rich_web_answers_reference_bottom_force",
  "cdxenablerenderforisocomp",
  "feature.EnableClientFileURLSupportForOfficeWebPaidCopilot",
  "feature.EnableDesignEditorImageGrounding",
  "feature.EnableDesignerEditor",
  "feature.EnableSkipRehydrationForSpeCIdImages",
  "rich_responses","feature.EnableBase64DataInMessageAnnotations",
  "EnableWorkIQToggle","feature.EnableExplicitWarmup",
  "feature.EnableSkipEmittingMessageOnFlush",
  "feature.EnableRemoveEmptySourceAttributions",
  "feature.EnableRemoveStreamingMode",
  "feature.OfficeWebToHelix","feature.OfficeDesktopToHelix",
  "feature.M365TeamsHubToHelix","feature.OwaHubToHelix",
  "feature.MonarchHubToHelix","feature.Win32OutlookHubToHelix",
  "feature.MacOutlookHubToHelix",
  "Agt_bizchat_enableGpt5ForHelix",
].join(",");

const CLIENT_INFO = {
  clientPlatform: "mcmcopilot-web",
  clientAppName: "Office",
  clientEntrypoint: "mcmcopilot-officeweb",
  ProductCategory: "Chat",
  clientAppType: "Web",
  productEntryPoint: "ChatPanel",
  deviceOS: "macOS",
  deviceType: "Desktop",
  clientPlatformVersion: "10.15.7",
};

const OPTIONS_SETS = [
  "search_result_progress_messages_with_search_queries",
  "enable_search_result_progress_messages",
  "update_textdoc_response_after_streaming",
  "deepleo_networking_timeout_10minutes_canmore",
  "cwc_flux_image","cwc_code_interpreter","cwc_code_interpreter_amsfix",
  "cwcfluxgptv","flux_v3_gptv_enable_upload_multi_image_in_turn_wo_ch",
  "gptvnorm2048","cwc_code_interpreter_citation_fix",
  "code_interpreter_interactive_charts",
  "cwc_code_interpreter_interactive_charts_inline_image",
  "code_interpreter_matplotlib_patching","cwc_fileupload_odb",
  "update_memory_plugin","add_custom_instructions",
  "cwc_flux_v3","flux_v3_progress_messages",
  "enable_batch_token_processing","enable_inferred_memory_read",
  "flux_v3_references","flux_v3_references_entities","flux_v3_references_ci",
  "add_filestore_filetype",
  "cwc_code_interpreter_citation_sourceannotations",
  "cdxcwc_code_interpreter_hallucinated_url_filter",
  "flux_v3_image_gen_enable_dimensions",
  "flux_v3_image_gen_enable_non_watermarked_storage",
  "flux_v3_image_gen_enable_icon_dimensions",
  "flux_v3_image_gen_enable_system_text_with_params",
  "flux_v3_image_gen_enable_designer_dimensions_meta_prompting_in_system_prompts",
  "flux_v3_image_gen_enable_story","rich_responses",
];

const ALLOWED_MESSAGE_TYPES = [
  "Chat","Suggestion","InternalSearchQuery","Disengaged",
  "InternalLoaderMessage","Progress","GeneratedCode",
  "RenderCardRequest","AdsQuery","SemanticSerp",
  "GenerateContentQuery","GenerateGraphicArt","SearchQuery",
  "ConfirmationCard","AuthError","DeveloperLogs",
  "TriggerPlugin","HintInvocation","MemoryUpdate",
  "EndOfRequest","TriggerConfirmation","ResumeInvokeAction",
  "ResumeUserInputRequest","TriggerUserInputRequest",
  "EscapeHatch","TriggerPluginAuth","ResumePluginAuth",
  "SideBySide","ReferencesListComplete","SwitchRespondingEndpoint",
];

// --- Helpers ---
function uuid() { return crypto.randomUUID(); }
function timestamp() { return new Date().toISOString(); }
function parseMessages(raw) {
  return raw.split(RECORD_SEPARATOR).filter(Boolean).map((s) => {
    try { return JSON.parse(s); } catch { return null; }
  }).filter(Boolean);
}
function writeMessage(msg) {
  return JSON.stringify(msg) + RECORD_SEPARATOR;
}

// --- Build Hub URL ---
function buildHubUrl(token, chatSessionId, conversationId) {
  const params = new URLSearchParams({
    chatsessionid: chatSessionId,
    XRoutingParameterSessionKey: chatSessionId,
    clientrequestid: chatSessionId,
    "X-SessionId": BROWSER_SESSION_ID,
    ...(conversationId && { ConversationId: conversationId }),
    access_token: token,
    variants: VARIANTS,
    source: '"officeweb"',
    product: "Office",
    agentHost: "Bizchat.FullScreen",
    licenseType: "Premium",
    isEdu: "false",
    agent: "web",
    scenario: "OfficeWebPaidCopilot",
  });
  return `wss://substrate.office.com/m365Copilot/Chathub/${USER_ID}@${TENANT_ID}?${params.toString()}`;
}

// --- Build Chat Payload ---
function buildChatPayload(text, opts = {}) {
  const chatSessionId = opts.chatSessionId || uuid();
  const clientSessionId = BROWSER_SESSION_ID;
  return {
    source: "officeweb",
    clientCorrelationId: chatSessionId,
    sessionId: clientSessionId,
    optionsSets: OPTIONS_SETS,
    streamingMode: "ConciseWithPadding",
    options: {},
    extraExtensionParameters: {},
    allowedMessageTypes: ALLOWED_MESSAGE_TYPES,
    sliceIds: [],
    threadLevelGptId: {},
    requestId: chatSessionId,
    traceId: chatSessionId,
    isStartOfSession: opts.isStartOfSession || false,
    clientInfo: { ...CLIENT_INFO, clientSessionId },
    message: {
      author: "user",
      inputMethod: "Keyboard",
      text,
      entityAnnotationTypes: ["People","File","Event","Email","TeamsMessage"],
      requestId: chatSessionId,
      locationInfo: { timeZoneOffset: 4, timeZone: "Asia/Dubai" },
      locale: "en-us",
      messageType: "Chat",
      experienceType: "Default",
      adaptiveCards: [],
      clientPreferences: {},
      connectedFederatedConnections: ["dummyId"],
      clientInfo: { ...CLIENT_INFO, clientSessionId },
    },
    plugins: [{ Id: "BingWebSearch", Source: "BuiltIn" }],
    isSbsSupported: true,
    tone: "Gpt_6_Sol_Reasoning",
    renderReferencesBehindEOS: true,
    disconnectBehavior: "continue",
  };
}

// --- CopilotSession ---
class CopilotSession {
  constructor(token, conversationId) {
    this.token = token;
    this.chatSessionId = uuid().replace(/-/g, "").substring(0, 32);
    this.conversationId = conversationId || "";
    this.ws = null;
    this.connected = false;
    this.handshakeDone = false;
    this.pingInterval = null;
    this.lastActivity = Date.now();
    this._handlers = {};
  }

  on(event, fn) {
    if (!this._handlers[event]) this._handlers[event] = [];
    this._handlers[event].push(fn);
  }
  emit(event, ...args) {
    (this._handlers[event] || []).forEach((fn) => fn(...args));
  }
  removeListener(event, fn) {
    if (!this._handlers[event]) return;
    this._handlers[event] = this._handlers[event].filter((f) => f !== fn);
  }

  connect() {
    return new Promise((resolve, reject) => {
      const url = buildHubUrl(this.token, this.chatSessionId, this.conversationId);
      this.ws = new WebSocket(url, { headers: CHROME_HEADERS, perMessageDeflate: true });

      this.ws.on("open", () => {
        this.ws.send(writeMessage({ protocol: "json", version: 1 }));
      });

      this.ws.on("message", (data) => {
        const messages = parseMessages(data.toString());
        for (const msg of messages) {
          if (!this.handshakeDone && !msg.type) {
            this.handshakeDone = true;
            this.connected = true;
            this.startPing();
            resolve();
            continue;
          }
          if (!this.handshakeDone) continue;

          if (msg.type === SR_PING) continue;

          if (msg.type === SR_INVOCATION && msg.target === "update") {
            const arg = msg.arguments?.[0];
            if (!arg) continue;
            if (arg.conversationId) this.conversationId = arg.conversationId;
            this.lastActivity = Date.now();

            const event = { type: "update", conversationId: this.conversationId };
            if (arg.messages) event.messages = arg.messages.filter((m) => m.author !== "user");
            if (arg.writeAtCursor !== undefined) event.delta = arg.writeAtCursor;
            if (arg.patches) event.patches = arg.patches;
            this.emit("update", event);
          }

          if (msg.type === SR_COMPLETION) {
            this.emit("completion", { invocationId: msg.invocationId });
          }

          if (msg.type === SR_INVOCATION_RESULT) {
            this.emit("result", { invocationId: msg.invocationId, item: msg.item });
          }

          if (msg.type === SR_CLOSE) {
            this.emit("close", { error: msg.error });
          }
        }
      });

      this.ws.on("error", (err) => reject(err));
      this.ws.on("close", (code) => {
        this.connected = false;
        this.stopPing();
      });
    });
  }

  startPing() {
    this.pingInterval = setInterval(() => {
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.ws.send(writeMessage({ type: SR_PING }));
      }
    }, 15000);
  }

  stopPing() {
    if (this.pingInterval) clearInterval(this.pingInterval);
  }

  sendChat(text, timeoutMs = 60000) {
    return new Promise((resolve, reject) => {
      if (!this.connected || !this.ws || this.ws.readyState !== WebSocket.OPEN) {
        return reject(new Error("Not connected"));
      }

      const invocationId = "0";
      const payload = buildChatPayload(text, {
        chatSessionId: this.chatSessionId,
        conversationId: this.conversationId,
        isStartOfSession: !this.conversationId,
      });

      let finalMessage = null;
      let resultReceived = false;
      const firstTokenTime = Date.now();
      let firstTokenRendered = false;

      const resolveWithResult = () => {
        if (resultReceived) return;
        resultReceived = true;
        cleanup();

        // Send final Metrics back to server (matches browser behavior)
        const now = new Date().toISOString();
        this.ws.send(writeMessage({
          type: SR_INVOCATION,
          target: "Metrics",
          arguments: [{
            Timestamps: {
              RequestSent: timestamp(),
              FirstServiceResponseReceived: timestamp(),
              FirstServiceResponseRendered: timestamp(),
              FirstTokenReceived: new Date(firstTokenTime).toISOString(),
              LastTokenReceived: now,
              FirstTokenRendered: new Date(firstTokenTime).toISOString(),
              SuggestionsRendered: now,
              SuggestionsReceived: now,
            },
            RenderedChunkMetrics: { ChunkCount: 1 },
            ReceivedTokenMetrics: { TokenCount: finalMessage?.text?.length || 0, CharCount: finalMessage?.text?.length || 0 },
          }],
        }));

        resolve({
          text: finalMessage?.text || "",
          messageId: finalMessage?.messageId,
          conversationId: this.conversationId,
          references: finalMessage?.references,
          sourceAttributions: finalMessage?.sourceAttributions,
          suggestedResponses: finalMessage?.suggestedResponses,
        });
      };

      const onUpdate = (event) => {
        if (event.messages) {
          for (const m of event.messages) {
            if (m.text && m.author === "bot" && m.messageType !== "Progress" && m.messageType !== "InternalLoaderMessage") {
              finalMessage = m;
              if (!firstTokenRendered) {
                firstTokenRendered = true;
              }
            }
          }
        }
      };

      const onResult = (evt) => {
        if (evt.invocationId !== String(invocationId)) return;
        // type:2 contains the full turn result with all messages
        if (evt.item?.messages) {
          for (const m of evt.item.messages) {
            if (m.text && m.author === "bot" && m.messageType !== "Progress" && m.messageType !== "InternalLoaderMessage") {
              finalMessage = m;
            }
          }
        }
        resolveWithResult();
      };

      const onCompletion = (evt) => {
        if (evt.invocationId === String(invocationId)) {
          resolveWithResult();
        }
      };

      const onClose = (evt) => {
        cleanup();
        reject(new Error("Connection closed: " + (evt.error || "unknown")));
      };

      const cleanup = () => {
        this.removeListener("update", onUpdate);
        this.removeListener("result", onResult);
        this.removeListener("completion", onCompletion);
        this.removeListener("close", onClose);
        clearTimeout(timer);
      };

      const timer = setTimeout(() => {
        cleanup();
        resolve({
          text: finalMessage?.text || "",
          messageId: finalMessage?.messageId,
          conversationId: this.conversationId,
          partial: true,
        });
      }, timeoutMs);

      this.on("update", onUpdate);
      this.on("result", onResult);
      this.on("completion", onCompletion);
      this.on("close", onClose);

      // Send TypingStarted first (real browser does this)
      this.ws.send(writeMessage({
        type: SR_INVOCATION,
        target: "send",
        arguments: [{ type: "ClientActivity", activityType: "TypingStarted" }],
      }));

      this.ws.send(writeMessage({
        type: SR_STREAM_INVOCATION,
        target: "chat",
        invocationId,
        arguments: [payload],
      }));

      this.ws.send(writeMessage({
        type: SR_INVOCATION,
        target: "Metrics",
        arguments: [{
          Timestamps: {
            ConnectionStart: timestamp(),
            ConnectionEstablished: timestamp(),
            UserInputStart: timestamp(),
            UserInputSubmit: timestamp(),
            RequestSent: timestamp(),
          },
        }],
      }));
    });
  }

  disconnect() {
    this.stopPing();
    if (this.ws) this.ws.close();
  }
}

// --- Express App ---
const app = express();
app.use(express.json());

// --- Auth Endpoints ---

// POST /auth/login - Get the browser auth URL
app.post("/auth/login", async (req, res) => {
  try {
    const url = await auth.getAuthUrl();
    res.json({
      authUrl: url,
      instructions: [
        "1. Open the authUrl in your browser",
        "2. Sign in with your Microsoft account",
        "3. After redirect, COPY the full URL from address bar",
        "4. POST to /auth/complete with that URL",
      ],
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /auth/complete - Exchange the redirect URL for tokens
app.post("/auth/complete", async (req, res) => {
  const { redirectUrl } = req.body;
  if (!redirectUrl) {
    return res.status(400).json({ error: "redirectUrl required" });
  }

  try {
    const result = await auth.completeAuth(redirectUrl);
    res.json({
      success: true,
      message: "Authentication complete!",
      expiresIn: result.expiresIn,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /auth/status - Check authentication status
app.get("/auth/status", (req, res) => {
  res.json(auth.getAuthInfo());
});

// POST /auth/logout - Clear stored tokens
app.post("/auth/logout", (req, res) => {
  auth.logout();
  res.json({ ok: true });
});

// --- Conversation Management Endpoints ---

// GET /conversations - List server-tracked conversations (persisted to disk)
app.get("/conversations", (req, res) => {
  const conversations = [];
  serverConversations.forEach((info, id) => {
    conversations.push({ conversationId: id, ...info });
  });
  res.json({
    count: conversations.length,
    conversations,
  });
});

// POST /conversations/delete - Delete specific conversations
// { conversationIds: ["id1", "id2"] } or { conversationId: "single-id" }
app.post("/conversations/delete", async (req, res) => {
  const { conversationIds, conversationId } = req.body;
  const ids = conversationIds || (conversationId ? [conversationId] : []);

  if (ids.length === 0) {
    return res.status(400).json({ error: "conversationIds or conversationId required" });
  }

  try {
    const result = await deleteConversations(ids);
    res.json(result);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /conversations/cleanup - Delete all server-tracked conversations
app.post("/conversations/cleanup", async (req, res) => {
  const ids = Array.from(serverConversations.keys());
  if (ids.length === 0) {
    return res.json({ deleted: 0, message: "No server-tracked conversations to clean up" });
  }

  try {
    const result = await deleteConversations(ids);
    res.json(result);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /conversations/cleanup - Delete all server-tracked conversations
app.post("/conversations/cleanup", async (req, res) => {
  const ids = Array.from(serverConversations.keys());
  if (ids.length === 0) {
    return res.json({ deleted: 0, message: "No server-tracked conversations to clean up" });
  }

  try {
    const result = await deleteConversations(ids);
    res.json(result);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// --- Chat Endpoint ---

// POST /chat - Send message to Copilot
// Token is optional if authenticated via /auth/login
app.post("/chat", async (req, res) => {
  const { message, conversationId, timeout, autoDelete } = req.body;
  if (!message) {
    return res.status(400).json({ error: "message required" });
  }

  // Get token: manual override or auto-refreshed
  let token = req.body.token;
  if (!token) {
    try {
      token = await auth.getValidToken();
    } catch (err) {
      return res.status(401).json({
        error: "Not authenticated",
        detail: err.message,
        loginUrl: "/auth/login",
      });
    }
  }

  try {
    const session = new CopilotSession(token, conversationId);
    await session.connect();
    const response = await session.sendChat(message, timeout || 60000);
    session.disconnect();

    // Track server-created conversations (new conversations only)
    if (!conversationId && response.conversationId) {
      trackConversation(response.conversationId, message);
    }

    // Auto-delete if requested (for bulk processing cleanup)
    if (autoDelete === true && response.conversationId) {
      try {
        await deleteConversations([response.conversationId]);
        response.deleted = true;
      } catch (err) {
        console.error("[chat] Auto-delete failed:", err.message);
        response.deleteError = err.message;
      }
    }

    res.json(response);
  } catch (err) {
    console.error("[chat] error:", err.message);
    res.status(500).json({ error: err.message });
  }
});

// --- Start ---
const PORT = process.env.PORT || 3000;
app.listen(PORT, () => {
  console.log(`M365 Copilot API running on http://localhost:${PORT}`);
  console.log("Endpoints:");
  console.log("  POST /auth/login              - Start browser PKCE authentication");
  console.log("  GET  /auth/status             - Check authentication status");
  console.log("  POST /auth/logout             - Clear stored tokens");
  console.log("  POST /chat                    - Send message {message, conversationId?, token?, autoDelete?}");
  console.log("  GET  /conversations           - List server-tracked conversations");
  console.log("  POST /conversations/delete    - Delete specific conversations {conversationIds: [...]}");
  console.log("  POST /conversations/cleanup   - Delete all server-tracked conversations");
  if (auth.isTokenValid()) {
    console.log("  [auth] Token loaded from cache");
  } else {
    console.log("  [auth] Not authenticated - call POST /auth/login to start");
  }
});
