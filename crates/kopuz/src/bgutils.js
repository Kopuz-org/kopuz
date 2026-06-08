/* eslint-disable */
"use strict";
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// dist/index.js
var index_exports = {};
__export(index_exports, {
  BG: () => core_exports,
  BGError: () => BGError,
  GOOG_API_KEY: () => GOOG_API_KEY,
  GOOG_BASE_URL: () => GOOG_BASE_URL,
  USER_AGENT: () => USER_AGENT,
  YT_BASE_URL: () => YT_BASE_URL,
  base64ToU8: () => base64ToU8,
  buildURL: () => buildURL,
  default: () => index_default,
  getHeaders: () => getHeaders,
  isBrowser: () => isBrowser,
  u8ToBase64: () => u8ToBase64
});
module.exports = __toCommonJS(index_exports);

// dist/core/index.js
var core_exports = {};
__export(core_exports, {
  BotGuardClient: () => BotGuardClient,
  Challenge: () => challengeFetcher_exports,
  PoToken: () => webPoClient_exports,
  WebPoMinter: () => WebPoMinter
});

// dist/core/challengeFetcher.js
var challengeFetcher_exports = {};
__export(challengeFetcher_exports, {
  create: () => create,
  descramble: () => descramble,
  parseChallengeData: () => parseChallengeData
});

// dist/utils/constants.js
var GOOG_BASE_URL = "https://jnn-pa.googleapis.com";
var YT_BASE_URL = "https://www.youtube.com";
var GOOG_API_KEY = "AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw";
var USER_AGENT = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36(KHTML, like Gecko)";

// dist/utils/helpers.js
var base64urlCharRegex = /[-_.]/g;
var base64urlToBase64Map = {
  "-": "+",
  _: "/",
  ".": "="
};
var BGError = class extends TypeError {
  constructor(message, info) {
    super(message);
    this.name = "BGError";
    if (info)
      this.info = info;
  }
};
function base64ToU8(base64) {
  let base64Mod;
  if (base64urlCharRegex.test(base64)) {
    base64Mod = base64.replace(base64urlCharRegex, function(match) {
      return base64urlToBase64Map[match];
    });
  } else {
    base64Mod = base64;
  }
  base64Mod = atob(base64Mod);
  return new Uint8Array([...base64Mod].map((char) => char.charCodeAt(0)));
}
function u8ToBase64(u8, base64url = false) {
  const result = btoa(String.fromCharCode(...u8));
  if (base64url) {
    return result.replace(/\+/g, "-").replace(/\//g, "_");
  }
  return result;
}
function isBrowser() {
  var _a, _b, _c;
  const isBrowser2 = typeof window !== "undefined" && typeof window.document !== "undefined" && typeof window.document.createElement !== "undefined" && typeof window.HTMLElement !== "undefined" && typeof window.navigator !== "undefined" && typeof window.getComputedStyle === "function" && typeof window.requestAnimationFrame === "function" && typeof window.matchMedia === "function";
  const hasValidWindow = (_c = (_b = (_a = Object.getOwnPropertyDescriptor(globalThis, "window")) == null ? void 0 : _a.get) == null ? void 0 : _b.toString().includes("[native code]")) != null ? _c : false;
  return isBrowser2 && hasValidWindow;
}
function getHeaders() {
  const headers = {
    "content-type": "application/json+protobuf",
    "x-goog-api-key": GOOG_API_KEY,
    "x-user-agent": "grpc-web-javascript/0.1"
  };
  if (!isBrowser()) {
    headers["user-agent"] = USER_AGENT;
  }
  return headers;
}
function buildURL(endpointName, useYouTubeAPI) {
  return `${useYouTubeAPI ? YT_BASE_URL : GOOG_BASE_URL}/${useYouTubeAPI ? "api/jnn/v1" : "$rpc/google.internal.waa.v1.Waa"}/${endpointName}`;
}

// dist/core/challengeFetcher.js
async function create(bgConfig, interpreterHash) {
  const requestKey = bgConfig.requestKey;
  if (!bgConfig.fetch)
    throw new BGError("[Challenge]: Fetch function not provided");
  const payload = [requestKey];
  if (interpreterHash)
    payload.push(interpreterHash);
  const response = await bgConfig.fetch(buildURL("Create", bgConfig.useYouTubeAPI), {
    method: "POST",
    headers: getHeaders(),
    body: JSON.stringify(payload)
  });
  if (!response.ok)
    throw new BGError("[Challenge]: Failed to fetch challenge", { status: response.status });
  const rawData = await response.json();
  return parseChallengeData(rawData);
}
function parseChallengeData(rawData) {
  let challengeData = [];
  if (rawData.length > 1 && typeof rawData[1] === "string") {
    const descrambled = descramble(rawData[1]);
    challengeData = JSON.parse(descrambled || "[]");
  } else if (rawData.length && typeof rawData[0] === "object") {
    challengeData = rawData[0];
  }
  const [messageId, wrappedScript, wrappedUrl, interpreterHash, program, globalName, , clientExperimentsStateBlob] = challengeData;
  const privateDoNotAccessOrElseSafeScriptWrappedValue = Array.isArray(wrappedScript) ? wrappedScript.find((value) => value && typeof value === "string") : null;
  const privateDoNotAccessOrElseTrustedResourceUrlWrappedValue = Array.isArray(wrappedUrl) ? wrappedUrl.find((value) => value && typeof value === "string") : null;
  return {
    messageId,
    interpreterJavascript: {
      privateDoNotAccessOrElseSafeScriptWrappedValue,
      privateDoNotAccessOrElseTrustedResourceUrlWrappedValue
    },
    interpreterHash,
    program,
    globalName,
    clientExperimentsStateBlob
  };
}
function descramble(scrambledChallenge) {
  const buffer = base64ToU8(scrambledChallenge);
  if (buffer.length)
    return new TextDecoder().decode(buffer.map((b) => b + 97));
}

// dist/core/webPoClient.js
var webPoClient_exports = {};
__export(webPoClient_exports, {
  decodeColdStartToken: () => decodeColdStartToken,
  generate: () => generate,
  generateColdStartToken: () => generateColdStartToken,
  generatePlaceholder: () => generatePlaceholder
});

// dist/core/botGuardClient.js
var BotGuardClient = class _BotGuardClient {
  constructor(options) {
    this.vmFunctions = {};
    this.userInteractionElement = options.userInteractionElement;
    this.vm = options.globalObj[options.globalName];
    this.program = options.program;
  }
  /**
   * Factory method to create and load a BotGuardClient instance.
   * @param options - Configuration options for the BotGuardClient.
   * @returns A promise that resolves to a loaded BotGuardClient instance.
   */
  static async create(options) {
    return await new _BotGuardClient(options).load();
  }
  async load() {
    if (!this.vm)
      throw new BGError("[BotGuardClient]: VM not found in the global object");
    if (!this.vm.a)
      throw new BGError("[BotGuardClient]: Could not load program");
    const vmFunctionsCallback = (asyncSnapshotFunction, shutdownFunction, passEventFunction, checkCameraFunction) => {
      Object.assign(this.vmFunctions, { asyncSnapshotFunction, shutdownFunction, passEventFunction, checkCameraFunction });
    };
    try {
      this.syncSnapshotFunction = await this.vm.a(this.program, vmFunctionsCallback, true, this.userInteractionElement, () => {
      }, [[], []])[0];
    } catch (error) {
      throw new BGError(`[BotGuardClient]: Failed to load program (${error.message})`);
    }
    return this;
  }
  /**
   * Takes a snapshot asynchronously.
   * @returns The snapshot result.
   * @example
   * ```ts
   * const result = await botguard.snapshot({
   *   contentBinding: {
   *     c: "a=6&a2=10&b=SZWDwKVIuixOp7Y4euGTgwckbJA&c=1729143849&d=1&t=7200&c1a=1&c6a=1&c6b=1&hh=HrMb5mRWTyxGJphDr0nW2Oxonh0_wl2BDqWuLHyeKLo",
   *     e: "ENGAGEMENT_TYPE_VIDEO_LIKE",
   *     encryptedVideoId: "P-vC09ZJcnM"
   *    }
   * });
   *
   * console.log(result);
   * ```
   */
  async snapshot(args) {
    return new Promise((resolve, reject) => {
      if (!this.vmFunctions.asyncSnapshotFunction)
        return reject(new BGError("[BotGuardClient]: Async snapshot function not found"));
      this.vmFunctions.asyncSnapshotFunction((response) => resolve(response), [
        args.contentBinding,
        args.signedTimestamp,
        args.webPoSignalOutput,
        args.skipPrivacyBuffer
      ]);
    });
  }
  /**
   * Takes a snapshot synchronously.
   * @returns The snapshot result.
   * @throws Error Throws an error if the synchronous snapshot function is not found.
   */
  async snapshotSynchronous(args) {
    if (!this.syncSnapshotFunction)
      throw new BGError("[BotGuardClient]: Sync snapshot function not found");
    return this.syncSnapshotFunction([
      args.contentBinding,
      args.signedTimestamp,
      args.webPoSignalOutput,
      args.skipPrivacyBuffer
    ]);
  }
  /**
   * Passes an event to the VM.
   * @throws Error Throws an error if the pass event function is not found.
   */
  passEvent(args) {
    if (!this.vmFunctions.passEventFunction)
      throw new BGError("[BotGuardClient]: Pass event function not found");
    this.vmFunctions.passEventFunction(args);
  }
  /**
   * Checks the "camera".
   * @throws Error Throws an error if the check camera function is not found.
   */
  checkCamera(args) {
    if (!this.vmFunctions.checkCameraFunction)
      throw new BGError("[BotGuardClient]: Check camera function not found");
    this.vmFunctions.checkCameraFunction(args);
  }
  /**
   * Shuts down the VM. Taking a snapshot after this will throw an error.
   * @throws Error Throws an error if the shutdown function is not found.
   */
  shutdown() {
    if (!this.vmFunctions.shutdownFunction)
      throw new BGError("[BotGuardClient]: Shutdown function not found");
    this.vmFunctions.shutdownFunction();
  }
};

// dist/core/webPoMinter.js
var WebPoMinter = class _WebPoMinter {
  constructor(mintCallback) {
    this.mintCallback = mintCallback;
  }
  static async create(integrityTokenResponse, webPoSignalOutput) {
    const getMinter = webPoSignalOutput[0];
    if (!getMinter)
      throw new BGError("PMD:Undefined");
    if (!integrityTokenResponse.integrityToken)
      throw new BGError("Failed to create WebPoMinter: No integrity token provided", integrityTokenResponse);
    const mintCallback = await getMinter(base64ToU8(integrityTokenResponse.integrityToken));
    if (!(mintCallback instanceof Function))
      throw new BGError("APF:Failed");
    return new _WebPoMinter(mintCallback);
  }
  async mintAsWebsafeString(identifier) {
    const result = await this.mint(identifier);
    return u8ToBase64(result, true);
  }
  async mint(identifier) {
    const result = await this.mintCallback(new TextEncoder().encode(identifier));
    if (!result)
      throw new BGError("YNJ:Undefined");
    if (!(result instanceof Uint8Array))
      throw new BGError("ODM:Invalid");
    return result;
  }
};

// dist/core/webPoClient.js
async function generate(args) {
  const { program, bgConfig, globalName } = args;
  const { identifier } = bgConfig;
  const botguard = await BotGuardClient.create({ program, globalName, globalObj: bgConfig.globalObj });
  const webPoSignalOutput = [];
  const botguardResponse = await botguard.snapshot({ webPoSignalOutput });
  const payload = [bgConfig.requestKey, botguardResponse];
  const integrityTokenResponse = await bgConfig.fetch(buildURL("GenerateIT", bgConfig.useYouTubeAPI), {
    method: "POST",
    headers: getHeaders(),
    body: JSON.stringify(payload)
  });
  const integrityTokenJson = await integrityTokenResponse.json();
  const [integrityToken, estimatedTtlSecs, mintRefreshThreshold, websafeFallbackToken] = integrityTokenJson;
  const integrityTokenData = {
    integrityToken,
    estimatedTtlSecs,
    mintRefreshThreshold,
    websafeFallbackToken
  };
  const webPoMinter = await WebPoMinter.create(integrityTokenData, webPoSignalOutput);
  const poToken = await webPoMinter.mintAsWebsafeString(identifier);
  return { poToken, integrityTokenData };
}
function generateColdStartToken(identifier, clientState) {
  const encodedIdentifier = new TextEncoder().encode(identifier);
  if (encodedIdentifier.length > 118)
    throw new BGError("DFO:Invalid", { identifier });
  const timestamp = Math.floor(Date.now() / 1e3);
  const randomKeys = [Math.floor(Math.random() * 256), Math.floor(Math.random() * 256)];
  const header = randomKeys.concat([
    0,
    clientState != null ? clientState : 1
  ], [
    timestamp >> 24 & 255,
    timestamp >> 16 & 255,
    timestamp >> 8 & 255,
    timestamp & 255
  ]);
  const packet = new Uint8Array(2 + header.length + encodedIdentifier.length);
  packet[0] = 34;
  packet[1] = header.length + encodedIdentifier.length;
  packet.set(header, 2);
  packet.set(encodedIdentifier, 2 + header.length);
  const payload = packet.subarray(2);
  const keyLength = randomKeys.length;
  for (let i = keyLength; i < payload.length; i++) {
    payload[i] ^= payload[i % keyLength];
  }
  return u8ToBase64(packet, true);
}
function generatePlaceholder(identifier, clientState) {
  return generateColdStartToken(identifier, clientState);
}
function decodeColdStartToken(token) {
  const packet = base64ToU8(token);
  const payloadLength = packet[1];
  const totalPacketLength = 2 + payloadLength;
  if (packet.length !== totalPacketLength)
    throw new BGError("Invalid packet length.", { packetLength: packet.length, expectedLength: totalPacketLength });
  const payload = packet.subarray(2);
  const keyLength = 2;
  for (let i = keyLength; i < payload.length; ++i) {
    payload[i] ^= payload[i % keyLength];
  }
  const keys = [payload[0], payload[1]];
  const unknownVal = payload[2];
  const clientState = payload[3];
  const timestamp = payload[4] << 24 | payload[5] << 16 | payload[6] << 8 | payload[7];
  const date = new Date(timestamp * 1e3);
  const identifier = new TextDecoder().decode(payload.subarray(8));
  return {
    identifier,
    timestamp,
    unknownVal,
    clientState,
    keys,
    date
  };
}

// dist/index.js
var index_default = core_exports;
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  BG,
  BGError,
  GOOG_API_KEY,
  GOOG_BASE_URL,
  USER_AGENT,
  YT_BASE_URL,
  base64ToU8,
  buildURL,
  getHeaders,
  isBrowser,
  u8ToBase64
});
//# sourceMappingURL=index.cjs.map
