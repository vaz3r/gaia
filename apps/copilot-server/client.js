const WebSocket = require("ws");
const https = require("https");
const crypto = require("crypto");
const readline = require("readline");

// --- Config from env or CLI ---
const BEARER_TOKEN = process.env.M365_TOKEN || process.argv[2] || "";
const CHAT_SESSION_ID = process.env.CHAT_SESSION_ID || process.argv[3] || "";
const CONVERSATION_ID = process.env.CONVERSATION_ID || process.argv[4] || "";

// --- SignalR Constants ---
const SR_INVOCATION = 1;
const SR_STREAM_ITEM = 2;
const SR_COMPLETION = 3;
const SR_STREAM_INVOCATION = 4;
const SR_PING = 6;
const SR_CLOSE = 7;
const RECORD_SEPARATOR = "\x1e";

// --- SignalR JSON Protocol ---
function parseMessages(raw) {
  return raw.split(RECORD_SEPARATOR).filter(Boolean).map((s) => {
    try { return JSON.parse(s); } catch { return null; }
  }).filter(Boolean);
}

function writeMessage(msg) {
  return JSON.stringify(msg) + RECORD_SEPARATOR;
}

// --- Helpers ---
function uuid() { return crypto.randomUUID(); }
function timestamp() { return new Date().toISOString(); }

// --- Build the Substrate SignalR URL ---
function buildHubUrl(token, sessionId, conversationId) {
  const userId = "68ce272e-092a-460f-b4a3-20ad9501f523";
  const tenantId = "544b0e7e-1de4-43f2-8d46-d9acd0f15d60";
  const variants = [
    "EnableMcpServerWidgets","feature.EnableMcpServerWidgets",
    "feature.EnableImageGenInsufficientTokensThrottled",
    "feature.EnableImageGenSystemCapacityThrottled",
    "feature.EnableLuForChatCIQ","feature.enableChatCIQPlugin",
    "EnableRequestPlugins","feature.EnableSensitivityLabels",
    "EnableUnsupportedUrlDetector","feature.IsCustomEngineCopilotEnabled",
    "feature.bizchatfluxv3","feature.enablechatpages",
    "feature.turnOnDARecommendation","feature.IsStreamingModeInChatRequestEnabled",
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
    "feature.EnableShoppingCashback","feature.EnableMergingPureDeltas",
    "feature.isExternalEmailEnabled","feature.isExcludedEmailEnabled",
    "feature.disabledisallowedmsgs",
    "feature.enableCitationsForSynthesisData",
    "feature.EnableConversationShareApis",
    "feature.EnableConversationShareApisForMsa",
    "feature.EnableGoodbyeDrainGate",
    "feature.enableGenerateGraphicArtOptionsSet","cdximagen",
    "feature.EnableUpdatedUXForConfirmationDialog",
    "feature.EnableContentApiandDocTypeHtmlInRichAnswers",
    "cdxgrounding_api_v2_rich_web_answers_reference_bottom_force",
    "cdxenablerenderforisocomp",
    "feature.EnableDesignEditorImageGrounding",
    "feature.EnableDesignerEditor",
    "feature.EnableSkipRehydrationForSpeCIdImages",
    "feature.sourcescontrolmainline",
    "feature.sourcescontrolmainlineal",
    "feature.EnableConnectorExecutionControlsAllowlist",
    "feature.EnableBizchatMainlineExecutionControlsResolution",
    "feature.EnableInlineAuthForFCCInMainline",
    "cdxentrecapvifluxv3","rich_responses",
    "feature.EnableBase64DataInMessageAnnotations",
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

  const params = new URLSearchParams({
    chatsessionid: sessionId,
    XRoutingParameterSessionKey: sessionId,
    clientrequestid: sessionId,
    "X-SessionId": sessionId,
    ...(conversationId && { ConversationId: conversationId }),
    access_token: token,
    variants,
    source: '"officeweb"',
    product: "Office",
    agentHost: "Bizchat.FullScreen",
    licenseType: "Premium",
    isEdu: "false",
    agent: "work",
    scenario: "officeweb",
  });

  return `wss://substrate.office.com/m365Copilot/Chathub/${userId}@${tenantId}?${params.toString()}`;
}

// --- Build the chat message payload ---
function buildChatMessage(text, opts = {}) {
  const requestId = uuid();
  const now = timestamp();

  return {
    source: "officeweb",
    clientCorrelationId: requestId,
    sessionId: opts.sessionId || "",
    optionsSets: [
      "search_result_progress_messages_with_search_queries",
      "enable_search_result_progress_messages",
      "update_textdoc_response_after_streaming",
      "deepleo_networking_timeout_10minutes_canmore",
      "cwc_flux_image","cwc_code_interpreter","cwc_code_interpreter_amsfix",
      "cwcfluxgptv","flux_v3_gptv_enable_upload_multi_image_in_turn_wo_ch",
      "gptvnorm2048","cwc_code_interpreter_citation_fix",
      "code_interpreter_interactive_charts","cwc_code_interpreter_interactive_charts_inline_image",
      "code_interpreter_matplotlib_patching","cwc_fileupload_odb",
      "update_memory_plugin","add_custom_instructions",
      "cwc_flux_v3","flux_v3_progress_messages",
      "enable_batch_token_processing","enable_inferred_memory_read",
      "flux_v3_references","flux_v3_references_entities","flux_v3_references_ci",
      "add_filestore_filetype","cwc_code_interpreter_citation_sourceannotations",
      "cdxcwc_code_interpreter_hallucinated_url_filter",
      "flux_v3_image_gen_enable_dimensions","flux_v3_image_gen_enable_non_watermarked_storage",
      "flux_v3_image_gen_enable_icon_dimensions",
      "flux_v3_image_gen_enable_system_text_with_params",
      "flux_v3_image_gen_enable_designer_dimensions_meta_prompting_in_system_prompts",
      "flux_v3_image_gen_enable_story","rich_responses",
    ],
    streamingMode: "ConciseWithPadding",
    options: {},
    extraExtensionParameters: {},
    allowedMessageTypes: [
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
    ],
    sliceIds: [],
    threadLevelGptId: {},
    requestId,
    traceId: requestId,
    isStartOfSession: opts.isStartOfSession || false,
    clientInfo: {
      clientPlatform: "mcmcopilot-web",
      clientAppName: "Office",
      clientEntrypoint: "mcmcopilot-officeweb",
      clientSessionId: opts.sessionId || "",
      ProductCategory: "Chat",
      clientAppType: "Web",
      productEntryPoint: "ChatPanel",
      deviceOS: "macOS",
      deviceType: "Desktop",
      clientPlatformVersion: "10.15.7",
    },
    message: {
      author: "user",
      inputMethod: "Keyboard",
      text,
      entityAnnotationTypes: ["People","File","Event","Email","TeamsMessage"],
      requestId,
      locationInfo: { timeZoneOffset: 4, timeZone: "Asia/Dubai" },
      locale: "en-us",
      messageType: "Chat",
      experienceType: "Default",
      adaptiveCards: [],
      clientPreferences: {},
      connectedFederatedConnections: ["dummyId"],
      clientInfo: {
        clientPlatform: "mcmcopilot-web",
        clientAppName: "Office",
        clientEntrypoint: "mcmcopilot-officeweb",
        clientSessionId: opts.sessionId || "",
        ProductCategory: "Chat",
        clientAppType: "Web",
        productEntryPoint: "ChatPanel",
        deviceOS: "macOS",
        deviceType: "Desktop",
        clientPlatformVersion: "10.15.7",
      },
    },
    plugins: [{ Id: "BingWebSearch", Source: "BuiltIn" }],
    isSbsSupported: true,
    tone: "Gpt_6_Sol_Reasoning",
    renderReferencesBehindEOS: true,
    disconnectBehavior: "continue",
  };
}

// --- ChatHubClient ---
class ChatHubClient {
  constructor(token, options = {}) {
    this.token = token;
    this.sessionId = options.sessionId || uuid().replace(/-/g, "").substring(0, 32);
    this.conversationId = options.conversationId || "";
    this.ws = null;
    this.connected = false;
    this.invocationId = 0;
    this.pingInterval = null;
    this.onMessage = options.onMessage || (() => {});
    this.onDelta = options.onDelta || (() => {});
    this.onProgress = options.onProgress || (() => {});
    this.onDone = options.onDone || (() => {});
    this.connectionStartTime = timestamp();
    this.connectionEstablishedTime = null;
    this.pendingRequests = new Map();
    this._handlers = {};
  }

  on(event, fn) {
    if (!this._handlers[event]) this._handlers[event] = [];
    this._handlers[event].push(fn);
  }

  emit(event, ...args) {
    (this._handlers[event] || []).forEach((fn) => fn(...args));
  }

  nextInvocationId() {
    return String(this.invocationId++);
  }

  connect() {
    const url = buildHubUrl(this.token, this.sessionId, this.conversationId);
    console.log("[hub] connecting to substrate...");

    this.ws = new WebSocket(url, {
      headers: {
        Origin: "https://m365.cloud.microsoft",
        "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36",
      },
      perMessageDeflate: true,
    });

    this.ws.on("open", () => {
      console.log("[hub] connected, sending handshake...");
      // SignalR handshake must be sent first
      this.ws.send(writeMessage({ protocol: "json", version: 1 }));
    });

    this.ws.on("message", (data) => {
      const raw = data.toString();
      const messages = parseMessages(raw);
      for (const msg of messages) {
        this.handleMessage(msg);
      }
    });

    this.ws.on("error", (err) => console.error("[hub] error:", err.message));
    this.ws.on("close", (code) => {
      console.log("[hub] closed:", code);
      this.connected = false;
      this.stopPing();
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

  handleMessage(msg) {
    // Handshake response (empty object = success)
    if (!this.connected && !msg.type) {
      this.connected = true;
      this.connectionEstablishedTime = timestamp();
      console.log("[hub] handshake OK");
      this.startPing();
      this.emit("ready");
      return;
    }

    switch (msg.type) {
      case SR_PING:
        break;

      case SR_INVOCATION: {
        // Server invocation — "update" carries chat messages/deltas
        if (msg.target === "update") {
          const arg = msg.arguments?.[0];
          if (!arg) break;

          // Full message(s)
          if (arg.messages) {
            for (const m of arg.messages) {
              if (m.author === "user") continue;
              if (m.messageType === "Progress") {
                this.onProgress(m);
              } else if (m.messageType === "InternalLoaderMessage") {
                // typing indicator, ignore
              } else if (m.messageType === "ReferencesListComplete") {
                // ignore
              } else {
                this.onMessage(m);
              }
            }
          }

          // Streaming delta
          if (arg.writeAtCursor !== undefined) {
            this.onDelta(arg.writeAtCursor);
          }

          // Conversation metadata
          if (arg.conversationId) {
            this.conversationId = arg.conversationId;
          }

          // Patches (spokenText updates etc)
          if (arg.patches) {
            for (const p of arg.patches) {
              if (p.path?.includes("spokenText") && p.value) {
                // spoken text update
              }
            }
          }
        }
        break;
      }

      case SR_COMPLETION:
        console.log("[hub] invocation", msg.invocationId, "completed");
        break;

      case SR_CLOSE:
        console.log("[hub] server closing:", msg.error);
        break;

      default:
        break;
    }
  }

  sendChat(text) {
    if (!this.connected) {
      console.error("[hub] not connected");
      return;
    }

    const invocationId = this.nextInvocationId();
    const payload = buildChatMessage(text, {
      sessionId: this.sessionId,
      conversationId: this.conversationId,
      isStartOfSession: !this.conversationId,
    });

    const msg = {
      type: SR_STREAM_INVOCATION,
      target: "chat",
      invocationId,
      arguments: [payload],
    };

    console.log("[hub] sending:", text.substring(0, 80));
    this.ws.send(writeMessage(msg));

    // Send metrics
    const metricsMsg = {
      type: SR_INVOCATION,
      target: "Metrics",
      arguments: [{
        Timestamps: {
          ConnectionStart: this.connectionStartTime,
          ConnectionEstablished: this.connectionEstablishedTime,
          UserInputStart: timestamp(),
          UserInputSubmit: timestamp(),
          RequestSent: timestamp(),
        },
      }],
    };
    this.ws.send(writeMessage(metricsMsg));
  }

  disconnect() {
    this.stopPing();
    if (this.ws) this.ws.close();
  }
}

// --- Trouter Client (optional, for fallback) ---
class TrouterClient {
  constructor(token) {
    this.token = token;
    this.ws = null;
    this.frameId = 0;
    this.id = null;
  }

  nextId() { return ++this.frameId; }

  buildUrl() {
    const params = new URLSearchParams({ cv: "2025.30.01.1", ua: "BizChat", hr: "", v: "3639/1.0.0" });
    return `wss://go.trouter.teams.microsoft.com/v4/c?timeout=40&epid=${uuid()}&ccid=&dom=m365.cloud.microsoft&cor_id=${uuid()}&con_num=${Date.now()}_0&tc=${encodeURIComponent(params.toString())}`;
  }

  parseFrame(raw) {
    const match = raw.match(/^(\d+):(\d+)(\+)?(?::(\d*))?::(.*)$/s);
    if (!match) return null;
    const [, type, id, , ackId, payload] = match;
    let parsed; try { parsed = JSON.parse(payload); } catch { parsed = payload; }
    return { type: +type, id: +id, ackId, payload: parsed };
  }

  buildFrame(type, id, payload, ackId) {
    return `${type}:${id}${ackId !== undefined ? `::${ackId}` : "::"}${JSON.stringify(payload)}`;
  }

  connect() {
    this.ws = new WebSocket(this.buildUrl(), {
      headers: {
        Origin: "https://m365.cloud.microsoft",
        "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36",
      },
      perMessageDeflate: true,
    });
    this.ws.on("open", () => {
      const id = this.nextId();
      this.ws.send(this.buildFrame(5, id, {
        name: "user.authenticate",
        args: [{ headers: { Authorization: `Bearer ${this.token}`, "X-MS-Migration": "True" } }],
      }));
    });
    this.ws.on("message", (data) => {
      const frame = this.parseFrame(data.toString());
      if (!frame) return;
      if (frame.type === 5 && frame.payload?.name === "ping") {
        this.ws.send(this.buildFrame(5, frame.id, { name: "ping" }, frame.id));
      }
      if (frame.type === 5 && frame.payload?.name === "trouter.connected") {
        this.id = frame.payload.args?.[0]?.id;
        console.log("[trouter] connected, id:", this.id);
      }
    });
    this.ws.on("error", (err) => console.error("[trouter] error:", err.message));
    this.ws.on("close", () => console.log("[trouter] closed"));
  }
}

// --- Main ---
async function main() {
  const token = BEARER_TOKEN;
  if (!token) {
    console.error("Usage: node client.js <bearer_token> [session_id] [conversation_id]");
    console.error("Or set M365_TOKEN, CHAT_SESSION_ID, CONVERSATION_ID env vars");
    process.exit(1);
  }

  console.log("=== M365 Copilot Client ===");
  console.log("Token:", token.substring(0, 30) + "...");

  // Connect trouter (auth)
  const trouter = new TrouterClient(token);
  trouter.connect();

  // Connect SignalR chat hub
  const chat = new ChatHubClient(token, {
    sessionId: CHAT_SESSION_ID,
    conversationId: CONVERSATION_ID,
    onMessage: (msg) => {
      console.log("\n[assistant]", msg.text);
    },
    onDelta: (text) => {
      process.stdout.write(text);
    },
    onProgress: (msg) => {
      process.stdout.write(`\r[progress] ${msg.text || "..."}`);
    },
    onDone: (item) => {
      if (item.conversationId) {
        console.log("\n[meta] conversationId:", item.conversationId);
      }
    },
  });

  chat.connect();

  // Wait for handshake before starting REPL
  chat.on("ready", () => {
    console.log("\nReady! Type your message below:\n");
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.on("line", (line) => {
      const text = line.trim();
      if (text.toLowerCase() === "exit") {
        chat.disconnect();
        trouter.ws?.close();
        process.exit(0);
      }
      if (text) chat.sendChat(text);
    });
  });
}

main().catch(console.error);
