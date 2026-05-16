import * as zip from "./zip.min.js";

const WASM_URL = new URL("./wemu.wasm", import.meta.url);
const SCHEDULER_FPS = 30;
const SCHEDULER_FRAME_INTERVAL_MS = 1000 / SCHEDULER_FPS;
const SCHEDULER_MICROSECONDS_PER_FRAME = Math.round(1_000_000 / SCHEDULER_FPS);
const IN_MEMORY_ZIP_LIMIT = 64 * 1024 * 1024;
const LOG_PREFIX = "[wemu]";
const SLOW_FRAME_LOG_MS = 100;
const ERR_DISP_FEATURE = "err_disp";
const ERR_DISP_SCHEMA_VERSION = 1;
const ERR_DISP_MAX_REPORT_BYTES = 128 * 1024;

const STOP_NAMES = new Map([
  [0, "Running"],
  [1, "ExitProcess"],
  [2, "MaxInstructions"],
  [3, "Breakpoint"],
  [4, "HleBooted"],
  [5, "CpuHalted"],
  [6, "FrontendQuit"],
  [7, "Waiting"],
]);
const REG_EAX = 0;
const REG_ECX = 1;
const REG_EDX = 2;
const REG_EBX = 3;
const REG_ESP = 4;
const REG_EBP = 5;
const REG_ESI = 6;
const REG_EDI = 7;
const VFS_READ = 1;
const VFS_WRITE = 2;
const ERROR_FILE_NOT_FOUND = 2;
const ERROR_ACCESS_DENIED = 5;
const FRAME_INPUT_RECORD_BYTES = 16;
const FRAME_EVENT_MOUSE_MOVE = 2;
const FRAME_EVENT_MOUSE_DOWN = 3;
const FRAME_EVENT_MOUSE_UP = 4;
const FRAME_EVENT_MOUSE_RIGHT_DOWN = 5;
const FRAME_EVENT_MOUSE_RIGHT_UP = 6;
const FRAME_EVENT_KEY_DOWN = 7;
const FRAME_EVENT_KEY_UP = 8;
const FRAME_EVENT_TEXT_CHAR = 9;
const KEY_NAME_TO_VK = new Map([
  ["Backspace", 0x08],
  ["Tab", 0x09],
  ["Enter", 0x0d],
  ["Shift", 0x10],
  ["Control", 0x11],
  ["Alt", 0x12],
  ["Escape", 0x1b],
  [" ", 0x20],
  ["ArrowLeft", 0x25],
  ["ArrowUp", 0x26],
  ["ArrowRight", 0x27],
  ["ArrowDown", 0x28],
  ["Delete", 0x2e],
  ["Home", 0x24],
  ["End", 0x23],
  ["F1", 0x70],
  ["F2", 0x71],
  ["F3", 0x72],
  ["F4", 0x73],
  ["F5", 0x74],
  ["F6", 0x75],
  ["F7", 0x76],
  ["F8", 0x77],
  ["F9", 0x78],
  ["F10", 0x79],
  ["F11", 0x7a],
  ["F12", 0x7b],
]);

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const errDispReportUrl = (() => {
  const params = new URLSearchParams(location.search);
  const queryUrl = params.get("err_disp_report");
  const metaUrl = document.querySelector("meta[name='wemu-err-disp-report-url']")?.content;
  const globalUrl = window.WEMU_ERR_DISP_REPORT_URL;
  const url = String(queryUrl || globalUrl || metaUrl || "").trim();
  return /^(0|false|off)$/i.test(url) ? "" : url;
})();
const gdiDecoder = (() => {
  const codepage = new URLSearchParams(location.search).get("codepage") || "big5";
  try {
    return new TextDecoder(codepage);
  } catch {
    return new TextDecoder("windows-1252");
  }
})();

const gdiTextRaster = {
  canvas: null,
  ctx: null,
  width: 0,
  height: 0,
};

function logInfo(message, detail) {
  if (detail === undefined) {
    console.info(`${LOG_PREFIX} ${message}`);
  } else {
    console.info(`${LOG_PREFIX} ${message}`, detail);
  }
}

function logWarn(message, detail) {
  if (detail === undefined) {
    console.warn(`${LOG_PREFIX} ${message}`);
  } else {
    console.warn(`${LOG_PREFIX} ${message}`, detail);
  }
}

function logError(message, err) {
  console.error(`${LOG_PREFIX} ${message}`, err);
}

function gdiRasterContext(width, height) {
  if (!gdiTextRaster.canvas) {
    gdiTextRaster.canvas = typeof OffscreenCanvas === "function"
      ? new OffscreenCanvas(width, height)
      : document.createElement("canvas");
    gdiTextRaster.ctx = gdiTextRaster.canvas.getContext("2d", { willReadFrequently: true });
  }
  if (gdiTextRaster.width !== width || gdiTextRaster.height !== height) {
    gdiTextRaster.canvas.width = width;
    gdiTextRaster.canvas.height = height;
    gdiTextRaster.width = width;
    gdiTextRaster.height = height;
  }
  return gdiTextRaster.ctx;
}

function colorrefRgb(colorref) {
  return {
    r: colorref & 0xff,
    g: (colorref >>> 8) & 0xff,
    b: (colorref >>> 16) & 0xff,
  };
}

function surfaceStride(bpp) {
  if (bpp <= 8) return 1;
  if (bpp <= 16) return 2;
  if (bpp <= 24) return 3;
  return 4;
}

function measureCanvasText(ctx, text, extra) {
  if (!extra) {
    return ctx.measureText(text).width;
  }
  let width = 0;
  let glyphs = 0;
  for (const ch of text) {
    if (glyphs) {
      width += extra;
    }
    width += ctx.measureText(ch).width;
    glyphs += 1;
  }
  return Math.max(0, width);
}

function fillCanvasText(ctx, text, x, y, extra) {
  if (!extra) {
    ctx.fillText(text, x, y);
    return;
  }
  let cursor = x;
  let glyphs = 0;
  for (const ch of text) {
    if (glyphs) {
      cursor += extra;
    }
    ctx.fillText(ch, cursor, y);
    cursor += ctx.measureText(ch).width;
    glyphs += 1;
  }
}

function blend(src, dst, alpha) {
  return Math.round((src * alpha + dst * (255 - alpha)) / 255);
}

function writeCanvasTextPixel(memory, offset, bpp, srcR, srcG, srcB, alpha, colorref) {
  if (!alpha || offset < 0 || offset + surfaceStride(bpp) > memory.length) {
    return;
  }
  if (bpp <= 8) {
    if (alpha > 64) {
      memory[offset] = Math.max(1, colorref & 0xff);
    }
    return;
  }
  if (bpp <= 16) {
    const old = memory[offset] | (memory[offset + 1] << 8);
    const dstR = ((old >>> 11) & 0x1f) * 255 / 31;
    const dstG = ((old >>> 5) & 0x3f) * 255 / 63;
    const dstB = (old & 0x1f) * 255 / 31;
    const r = blend(srcR, dstR, alpha);
    const g = blend(srcG, dstG, alpha);
    const b = blend(srcB, dstB, alpha);
    const packed = ((r >>> 3) << 11) | ((g >>> 2) << 5) | (b >>> 3);
    memory[offset] = packed & 0xff;
    memory[offset + 1] = packed >>> 8;
    return;
  }
  const dstB = memory[offset];
  const dstG = memory[offset + 1];
  const dstR = memory[offset + 2];
  memory[offset] = blend(srcB, dstB, alpha);
  memory[offset + 1] = blend(srcG, dstG, alpha);
  memory[offset + 2] = blend(srcR, dstR, alpha);
  if (bpp > 24) {
    memory[offset + 3] = 0;
  }
}

function drawCanvasTextToSurface(
  api,
  surfaceBuffer,
  surfaceWidth,
  surfaceHeight,
  pitch,
  bpp,
  textPtr,
  textLen,
  x,
  y,
  fontHeight,
  extra,
  colorref,
  clipLeft,
  clipTop,
  clipRight,
  clipBottom,
) {
  if (!api || !textLen || !surfaceWidth || !surfaceHeight) {
    return 0;
  }
  const ctx = gdiRasterContext(surfaceWidth, surfaceHeight);
  const memory = new Uint8Array(api.memory.buffer);
  const text = gdiDecoder.decode(new Uint8Array(api.memory.buffer, textPtr, textLen));
  const { r, g, b } = colorrefRgb(colorref);
  const size = Math.max(1, Math.min(fontHeight | 0, 200));
  const leftClip = Math.max(0, Math.min(surfaceWidth, clipLeft | 0));
  const topClip = Math.max(0, Math.min(surfaceHeight, clipTop | 0));
  const rightClip = Math.max(leftClip, Math.min(surfaceWidth, clipRight | 0));
  const bottomClip = Math.max(topClip, Math.min(surfaceHeight, clipBottom | 0));
  if (rightClip <= leftClip || bottomClip <= topClip) {
    return 1;
  }

  ctx.font = `${size}px "MingLiU", "PMingLiU", "SimSun", sans-serif`;
  ctx.textBaseline = "top";
  ctx.textAlign = "left";
  ctx.fillStyle = `rgb(${r} ${g} ${b})`;
  const width = Math.ceil(measureCanvasText(ctx, text, extra | 0)) + 4;
  const copyLeft = Math.max(leftClip, Math.floor(x) - 2);
  const copyTop = Math.max(topClip, Math.floor(y) - 2);
  const copyRight = Math.min(rightClip, Math.ceil(x + width) + 2);
  const copyBottom = Math.min(bottomClip, Math.ceil(y + size * 1.35) + 2);
  if (copyRight <= copyLeft || copyBottom <= copyTop) {
    return 1;
  }

  ctx.clearRect(copyLeft, copyTop, copyRight - copyLeft, copyBottom - copyTop);
  ctx.save();
  ctx.beginPath();
  ctx.rect(leftClip, topClip, rightClip - leftClip, bottomClip - topClip);
  ctx.clip();
  fillCanvasText(ctx, text, x, y, extra | 0);
  ctx.restore();

  const image = ctx.getImageData(copyLeft, copyTop, copyRight - copyLeft, copyBottom - copyTop);
  const data = image.data;
  const stride = surfaceStride(bpp);
  for (let row = 0; row < image.height; row += 1) {
    for (let col = 0; col < image.width; col += 1) {
      const src = (row * image.width + col) * 4;
      const alpha = data[src + 3];
      if (!alpha) {
        continue;
      }
      const dst = surfaceBuffer + (copyTop + row) * pitch + (copyLeft + col) * stride;
      writeCanvasTextPixel(memory, dst, bpp, data[src], data[src + 1], data[src + 2], alpha, colorref);
    }
  }
  return 1;
}

function eventKeyToVk(key) {
  if (KEY_NAME_TO_VK.has(key)) {
    return KEY_NAME_TO_VK.get(key);
  }
  if (key.length === 1) {
    const upper = key.toUpperCase();
    if (/^[A-Z0-9]$/.test(upper)) {
      return upper.charCodeAt(0);
    }
  }
  return null;
}

function isTextKey(event) {
  return event.key.length === 1 && !event.ctrlKey && !event.altKey && !event.metaKey;
}

async function loadWasmModule(onStatus) {
  onStatus("Loading emulator");
  const started = performance.now();
  logInfo("loading wasm", { url: String(WASM_URL) });
  const response = await fetch(WASM_URL);
  if (!response.ok) {
    throw new Error(`fetch ${WASM_URL}: HTTP ${response.status}`);
  }
  logInfo("wasm fetched", {
    status: response.status,
    bytes: Number(response.headers.get("content-length")) || null,
  });
  if (WebAssembly.compileStreaming) {
    try {
      onStatus("Preparing emulator");
      const module = await WebAssembly.compileStreaming(response.clone());
      logInfo("wasm compiled", { streaming: true, ms: Math.round(performance.now() - started) });
      return module;
    } catch (err) {
      if (!/mime|content-type/i.test(String(err))) {
        throw err;
      }
      logWarn("wasm compileStreaming fallback", String(err));
    }
  }
  onStatus("Preparing emulator");
  const module = await WebAssembly.compile(await response.arrayBuffer());
  logInfo("wasm compiled", { streaming: false, ms: Math.round(performance.now() - started) });
  return module;
}

function wasmImports(getApi) {
  return {
    env: {
      wemu_console_log(ptr, len) {
        const api = getApi();
        if (api) {
          console.log(`${LOG_PREFIX} ${decoder.decode(new Uint8Array(api.memory.buffer, ptr, len))}`);
        }
      },
      wemu_now_ms() {
        return performance.now();
      },
      wemu_canvas_text(...args) {
        return drawCanvasTextToSurface(getApi(), ...args);
      },
    },
  };
}

class Wemu {
  static preload(onStatus = () => {}) {
    if (!Wemu.modulePromise) {
      Wemu.modulePromise = loadWasmModule(onStatus);
    }
    return Wemu.modulePromise;
  }

  static async create() {
    const started = performance.now();
    const module = await Wemu.preload();
    let api = null;
    const instance = await WebAssembly.instantiate(module, wasmImports(() => api));
    api = instance.exports;
    const handle = api.wemu_new(640, 480);
    if (!handle) {
      throw new Error("wemu_new failed");
    }
    const timingRc = api.wemu_set_frontend_timing(
      handle,
      SCHEDULER_FPS,
      SCHEDULER_MICROSECONDS_PER_FRAME,
    );
    if (timingRc < 0) {
      throw new Error("wemu_set_frontend_timing failed");
    }
    logInfo("emulator created", { handle, ms: Math.round(performance.now() - started) });
    return new Wemu(api, handle);
  }

  constructor(api, handle) {
    this.api = api;
    this.handle = handle;
    this.vfs = null;
    this.frameInput = [];
  }

  destroy() {
    if (this.vfs?.close) {
      this.vfs.close();
    }
    this.vfs = null;
    if (this.handle) {
      this.api.wemu_destroy(this.handle);
      this.handle = 0;
    }
  }

  check(rc, label) {
    if (rc < 0) {
      const detail = this.readString(
        this.api.wemu_last_error_ptr(this.handle),
        this.api.wemu_last_error_len(this.handle),
      );
      throw new Error(detail ? `${label}: ${detail}` : `${label} failed`);
    }
    return rc;
  }

  addFile(guestPath, bytes) {
    this.withOwnedBytes(bytes, (dataPtr, dataLen) => {
      this.withString(guestPath, (pathPtr, pathLen) => {
        this.check(
          this.api.wemu_add_file_owned(this.handle, pathPtr, pathLen, dataPtr, dataLen),
          `add ${guestPath}`,
        );
      });
    });
  }

  addAsyncFile(guestPath, size, writable = false) {
    const lo = size >>> 0;
    const hi = Math.floor(size / 0x1_0000_0000) >>> 0;
    this.withString(guestPath, (pathPtr, pathLen) => {
      this.check(
        this.api.wemu_add_async_file(
          this.handle,
          pathPtr,
          pathLen,
          lo,
          hi,
          writable ? 1 : 0,
        ),
        `add async ${guestPath}`,
      );
    });
  }

  enableAsyncVfsWrites() {
    this.check(this.api.wemu_enable_async_vfs_writes(this.handle), "enable async VFS writes");
  }

  guestPathKey(guestPath) {
    return this.withString(guestPath, (pathPtr, pathLen) => {
      this.check(this.api.wemu_guest_path_key(this.handle, pathPtr, pathLen), `key ${guestPath}`);
      return this.readString(
        this.api.wemu_blob_ptr(this.handle),
        this.api.wemu_blob_len(this.handle),
      );
    });
  }

  setVfs(vfs) {
    this.vfs = vfs;
  }

  loadExe(guestPath, bytes) {
    const started = performance.now();
    this.withBytes(bytes, (exePtr, exeLen) => {
      this.withString(guestPath, (pathPtr, pathLen) => {
        this.check(
          this.api.wemu_load_exe(this.handle, pathPtr, pathLen, exePtr, exeLen),
          "load exe",
        );
      });
    });
    logInfo("exe loaded", {
      path: guestPath,
      bytes: bytes.length,
      ms: Math.round(performance.now() - started),
    });
  }

  runFor(insns) {
    return this.check(this.api.wemu_run_for(this.handle, insns), "run");
  }

  async runOneFrame() {
    const started = performance.now();
    const input = this.takeFrameInputBytes();
    const stop = this.withBytes(input, (ptr, len) => (
      this.check(this.api.wemu_run_one_frame(this.handle, ptr, len), "run frame")
    ));
    await this.serviceVfsRequest();
    const ms = performance.now() - started;
    if (ms >= SLOW_FRAME_LOG_MS && performance.now() - lastSlowFrameLogAt >= 1000) {
      lastSlowFrameLogAt = performance.now();
      logWarn("slow frame", { ms: Math.round(ms), stop });
    }
    return stop;
  }

  pendingVfsRequest() {
    const id = this.api.wemu_pending_vfs_request_id(this.handle);
    const kind = this.api.wemu_pending_vfs_request_kind(this.handle);
    if (!id || !kind) {
      return null;
    }
    const lo = BigInt(this.api.wemu_pending_vfs_request_offset_lo(this.handle) >>> 0);
    const hi = BigInt(this.api.wemu_pending_vfs_request_offset_hi(this.handle) >>> 0);
    const dataPtr = this.api.wemu_pending_vfs_request_data_ptr(this.handle);
    const dataLen = this.api.wemu_pending_vfs_request_data_len(this.handle);
    const data = dataPtr && dataLen
      ? new Uint8Array(new Uint8Array(this.api.memory.buffer, dataPtr, dataLen))
      : new Uint8Array(0);
    return {
      id,
      kind,
      path: this.readString(
        this.api.wemu_pending_vfs_request_path_ptr(this.handle),
        this.api.wemu_pending_vfs_request_path_len(this.handle),
      ),
      offset: Number((hi << 32n) | lo),
      len: this.api.wemu_pending_vfs_request_len(this.handle),
      data,
    };
  }

  completeVfsRequest(id, status, transferred, data = new Uint8Array(0)) {
    this.withBytes(data, (ptr, len) => {
      this.check(
        this.api.wemu_complete_vfs_request(this.handle, id, status, transferred, ptr, len),
        `complete VFS request ${id}`,
      );
    });
  }

  async serviceVfsRequest() {
    const request = this.pendingVfsRequest();
    if (!request) {
      return;
    }
    if (request.kind === VFS_READ) {
      const data = this.vfs ? await this.vfs.read(request.path, request.offset, request.len) : null;
      if (!data) {
        logWarn("vfs read miss", {
          id: request.id,
          path: request.path,
          offset: request.offset,
          len: request.len,
        });
        this.completeVfsRequest(request.id, ERROR_FILE_NOT_FOUND, 0);
        return;
      }
      this.completeVfsRequest(request.id, 0, data.length, data);
      return;
    }
    if (request.kind === VFS_WRITE) {
      if (!this.vfs) {
        logWarn("vfs write denied without mounted vfs", {
          id: request.id,
          path: request.path,
          offset: request.offset,
          len: request.data.length,
        });
        this.completeVfsRequest(request.id, ERROR_ACCESS_DENIED, 0);
        return;
      }
      const written = await this.vfs.write(request.path, request.offset, request.data);
      this.completeVfsRequest(request.id, 0, written);
      return;
    }
    logWarn("unknown vfs request", { id: request.id, kind: request.kind, path: request.path });
    this.completeVfsRequest(request.id, ERROR_ACCESS_DENIED, 0);
  }

  mouseMove(x, y) {
    this.queueFrameInput(FRAME_EVENT_MOUSE_MOVE, x, y);
  }

  mouseDown(x, y) {
    this.queueFrameInput(FRAME_EVENT_MOUSE_DOWN, x, y);
  }

  mouseUp(x, y) {
    this.queueFrameInput(FRAME_EVENT_MOUSE_UP, x, y);
  }

  mouseRightDown(x, y) {
    this.queueFrameInput(FRAME_EVENT_MOUSE_RIGHT_DOWN, x, y);
  }

  mouseRightUp(x, y) {
    this.queueFrameInput(FRAME_EVENT_MOUSE_RIGHT_UP, x, y);
  }

  keyDown(vk) {
    this.queueFrameInput(FRAME_EVENT_KEY_DOWN, vk);
  }

  keyUp(vk) {
    this.queueFrameInput(FRAME_EVENT_KEY_UP, vk);
  }

  text(value) {
    for (const ch of value) {
      this.queueFrameInput(FRAME_EVENT_TEXT_CHAR, ch.codePointAt(0));
    }
  }

  queueFrameInput(kind, a = 0, b = 0, c = 0) {
    if (kind === FRAME_EVENT_MOUSE_MOVE && this.frameInput.length >= 4) {
      const index = this.frameInput.length - 4;
      if (this.frameInput[index] === FRAME_EVENT_MOUSE_MOVE) {
        this.frameInput[index + 1] = a >>> 0;
        this.frameInput[index + 2] = b >>> 0;
        this.frameInput[index + 3] = c >>> 0;
        return;
      }
    }
    this.frameInput.push(kind >>> 0, a >>> 0, b >>> 0, c >>> 0);
  }

  takeFrameInputBytes() {
    const words = this.frameInput;
    this.frameInput = [];
    const bytes = new Uint8Array((words.length / 4) * FRAME_INPUT_RECORD_BYTES);
    const view = new DataView(bytes.buffer);
    for (let i = 0; i < words.length; i++) {
      view.setUint32(i * 4, words[i] >>> 0, true);
    }
    return bytes;
  }

  frame() {
    const width = this.api.wemu_width(this.handle);
    const height = this.api.wemu_height(this.handle);
    const ptr = this.api.wemu_framebuffer_ptr(this.handle);
    const len = this.api.wemu_framebuffer_len(this.handle);
    const rgba = new Uint8ClampedArray(this.api.memory.buffer, ptr, len);
    return { width, height, image: new ImageData(new Uint8ClampedArray(rgba), width, height) };
  }

  pngBlob() {
    this.check(this.api.wemu_screenshot_png(this.handle), "screenshot");
    const ptr = this.api.wemu_blob_ptr(this.handle);
    const len = this.api.wemu_blob_len(this.handle);
    const bytes = new Uint8Array(this.api.memory.buffer, ptr, len);
    return new Blob([new Uint8Array(bytes)], { type: "image/png" });
  }

  output() {
    return this.readString(
      this.api.wemu_output_ptr(this.handle),
      this.api.wemu_output_len(this.handle),
    );
  }

  lastError() {
    return this.readString(
      this.api.wemu_last_error_ptr(this.handle),
      this.api.wemu_last_error_len(this.handle),
    );
  }

  lastHle() {
    return this.readString(
      this.api.wemu_last_hle_ptr(this.handle),
      this.api.wemu_last_hle_len(this.handle),
    );
  }

  missingHleReport() {
    return this.readString(
      this.api.wemu_missing_hle_report_ptr(this.handle),
      this.api.wemu_missing_hle_report_len(this.handle),
    );
  }

  stats() {
    const lo = BigInt(this.api.wemu_insns_lo(this.handle) >>> 0);
    const hi = BigInt(this.api.wemu_insns_hi(this.handle) >>> 0);
    const regs = {
      eax: this.api.wemu_reg(this.handle, REG_EAX),
      ecx: this.api.wemu_reg(this.handle, REG_ECX),
      edx: this.api.wemu_reg(this.handle, REG_EDX),
      ebx: this.api.wemu_reg(this.handle, REG_EBX),
      esp: this.api.wemu_reg(this.handle, REG_ESP),
      ebp: this.api.wemu_reg(this.handle, REG_EBP),
      esi: this.api.wemu_reg(this.handle, REG_ESI),
      edi: this.api.wemu_reg(this.handle, REG_EDI),
    };
    return {
      stop: this.api.wemu_stop_reason(this.handle),
      exitCode: this.api.wemu_exit_code(this.handle),
      insns: ((hi << 32n) | lo).toString(),
      eip: this.api.wemu_eip(this.handle),
      eflags: this.api.wemu_eflags(this.handle),
      ...regs,
      regs,
    };
  }

  withString(text, fn) {
    return this.withBytes(encoder.encode(text), fn);
  }

  withBytes(bytes, fn) {
    const ptr = this.api.wemu_alloc(bytes.length);
    if (!ptr && bytes.length) {
      throw new Error(`wemu_alloc(${bytes.length}) failed`);
    }
    new Uint8Array(this.api.memory.buffer, ptr, bytes.length).set(bytes);
    try {
      return fn(ptr, bytes.length);
    } finally {
      this.api.wemu_free(ptr, bytes.length);
    }
  }

  withOwnedBytes(bytes, fn) {
    const ptr = this.api.wemu_alloc(bytes.length);
    if (!ptr && bytes.length) {
      throw new Error(`wemu_alloc(${bytes.length}) failed`);
    }
    new Uint8Array(this.api.memory.buffer, ptr, bytes.length).set(bytes);
    let owned = true;
    try {
      const result = fn(ptr, bytes.length);
      owned = false;
      return result;
    } finally {
      if (owned) {
        this.api.wemu_free(ptr, bytes.length);
      }
    }
  }

  readString(ptr, len) {
    if (!ptr || !len) {
      return "";
    }
    return decoder.decode(new Uint8Array(this.api.memory.buffer, ptr, len));
  }
}

const MOUNT_ROOT = "C:\\mnt";

const els = {
  zipFile: document.getElementById("zipFile"),
  loadButton: document.getElementById("loadButton"),
  idleLoadButton: document.getElementById("idleLoadButton"),
  pauseButton: document.getElementById("pauseButton"),
  shotButton: document.getElementById("shotButton"),
  status: document.getElementById("status"),
  statusDetail: document.getElementById("statusDetail"),
  progress: document.getElementById("progress"),
  currentGame: document.getElementById("currentGame"),
  stopValue: document.getElementById("stopValue"),
  insnsValue: document.getElementById("insnsValue"),
  eipValue: document.getElementById("eipValue"),
  eaxValue: document.getElementById("eaxValue"),
  espValue: document.getElementById("espValue"),
  guestOutput: document.getElementById("guestOutput"),
  screen: document.getElementById("screen"),
  exeDialog: document.getElementById("exeDialog"),
  exeChoices: document.getElementById("exeChoices"),
  exeCancel: document.getElementById("exeCancel"),
  errDispDialog: document.getElementById("errDispDialog"),
  errDispTitle: document.getElementById("errDispTitle"),
  errDispSummary: document.getElementById("errDispSummary"),
  errDispJson: document.getElementById("errDispJson"),
  errDispReportStatus: document.getElementById("errDispReportStatus"),
  errDispReportButton: document.getElementById("errDispReportButton"),
  errDispClose: document.getElementById("errDispClose"),
};

const ctx = els.screen.getContext("2d", { alpha: false });
let runtime = null;
let running = false;
let runFrame = 0;
let runFrameBusy = false;
let nextFrameAt = 0;
let lastSlowFrameLogAt = 0;
let wasmReady = false;
let busy = true;
let exeChoiceResolve = null;
let currentApp = null;
let lastErrDispReport = null;
const useTouchEvents = "ontouchstart" in window || typeof TouchEvent !== "undefined";
const pointerButtons = new Map();

function setCanvasModeSize(width, height) {
  width = Math.max(1, width | 0);
  height = Math.max(1, height | 0);
  if (els.screen.width !== width || els.screen.height !== height) {
    els.screen.width = width;
    els.screen.height = height;
  }
  els.screen.style.setProperty("--screen-aspect", String(width / height));
  els.screen.style.setProperty("--screen-ratio", `${width} / ${height}`);
}

function scheduleRun() {
  if (runFrame) {
    return;
  }
  runFrame = requestAnimationFrame(runChunk);
}

function stopRunLoop() {
  if (runFrame) {
    cancelAnimationFrame(runFrame);
    runFrame = 0;
  }
}

function setStatus(text, detail = "", error = false) {
  if (els.status) {
    els.status.textContent = text;
    els.status.classList.toggle("error", error);
  }
  if (els.statusDetail) {
    els.statusDetail.textContent = detail;
    els.statusDetail.classList.toggle("error", error);
  }
}

function setBusy(value, progress = 0) {
  busy = value;
  document.body.classList.toggle("is-busy", busy);
  if (els.progress) {
    els.progress.hidden = !busy;
    els.progress.value = Math.max(0, Math.min(1, progress));
  }
  updateButtons();
}

function updateButtons() {
  if (els.loadButton) {
    els.loadButton.disabled = busy || !wasmReady;
  }
  if (els.idleLoadButton) {
    els.idleLoadButton.disabled = busy || !wasmReady;
  }
  if (els.pauseButton) {
    els.pauseButton.disabled = !runtime || busy;
    els.pauseButton.textContent = running ? "Pause" : "Resume";
  }
  if (els.shotButton) {
    els.shotButton.disabled = !runtime || busy;
  }
  document.body.classList.toggle("has-runtime", Boolean(runtime));
}

function hex32(value) {
  return (value >>> 0).toString(16).padStart(8, "0");
}

function stopLabel(code, exitCode = 0) {
  const label = STOP_NAMES.get(code) ?? `Unknown(${code})`;
  return code === 1 ? `${label}(${exitCode})` : label;
}

function errDispStopKind(code) {
  switch (code) {
    case 1:
      return "process_exit";
    case 2:
      return "max_instructions";
    case 3:
      return "breakpoint";
    case 4:
      return "hle_booted";
    case 5:
      return "cpu_halted";
    case 6:
      return "frontend_quit";
    case 7:
      return "waiting";
    default:
      return "stopped";
  }
}

function errDispHasMissingHle(report) {
  return Boolean(report?.runtime?.missingHle) || /missing HLE/i.test(report?.message || "");
}

function errDispIsCpuError(report) {
  const message = report?.message || "";
  const wasmLastError = report?.runtime?.wasmLastError || "";
  return report?.kind === "cpu_halted" ||
    /(^|:\s*)cpu error:/i.test(message) ||
    /(^|:\s*)cpu error:/i.test(wasmLastError);
}

function errDispTitle(report) {
  if (errDispHasMissingHle(report)) {
    return "HLE API Not Implemented";
  }
  if (report?.kind === "process_exit") {
    return "Process Exited";
  }
  if (errDispIsCpuError(report)) {
    return "CPU Error";
  }
  return ERR_DISP_FEATURE;
}

function safeRead(fn, fallback = null) {
  try {
    return fn();
  } catch {
    return fallback;
  }
}

function parseJsonOrNull(text) {
  if (!text) {
    return null;
  }
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function errorToJson(err) {
  if (!err) {
    return null;
  }
  return {
    name: err.name || err.constructor?.name || "Error",
    message: err.message || String(err),
    stack: err.stack || "",
  };
}

function cpuReport(stats) {
  if (!stats) {
    return null;
  }
  return {
    eip: hex32(stats.eip),
    eflags: hex32(stats.eflags),
    regs: {
      eax: hex32(stats.eax),
      ecx: hex32(stats.ecx),
      edx: hex32(stats.edx),
      ebx: hex32(stats.ebx),
      esp: hex32(stats.esp),
      ebp: hex32(stats.ebp),
      esi: hex32(stats.esi),
      edi: hex32(stats.edi),
    },
  };
}

function browserReport() {
  return {
    href: `${location.origin}${location.pathname}`,
    userAgent: navigator.userAgent,
    platform: navigator.platform || "",
    language: navigator.language || "",
    pointerEvent: Boolean(window.PointerEvent),
    touchEvent: useTouchEvents,
    webAssembly: Boolean(window.WebAssembly),
    serviceWorker: "serviceWorker" in navigator,
  };
}

function runtimeReport(runtimeValue) {
  if (!runtimeValue) {
    return null;
  }
  const stats = safeRead(() => runtimeValue.stats());
  const wasmLastError = safeRead(() => runtimeValue.lastError(), "");
  const lastHle = safeRead(() => runtimeValue.lastHle(), "");
  const missingHleText = safeRead(() => runtimeValue.missingHleReport(), "");
  return {
    stop: stats ? {
      code: stats.stop,
      name: STOP_NAMES.get(stats.stop) ?? `Unknown(${stats.stop})`,
      exitCode: stats.exitCode,
    } : null,
    insns: stats?.insns ?? null,
    cpu: cpuReport(stats),
    lastHle,
    wasmLastError,
    missingHle: parseJsonOrNull(missingHleText) || missingHleText || null,
    guestOutput: safeRead(() => runtimeValue.output(), ""),
  };
}

function errDispReport(kind, message, runtimeValue, err = null, extra = {}) {
  const stats = safeRead(() => runtimeValue?.stats());
  return {
    v: ERR_DISP_SCHEMA_VERSION,
    feature: ERR_DISP_FEATURE,
    kind,
    message,
    stop: stats ? {
      code: stats.stop,
      name: stopLabel(stats.stop, stats.exitCode),
      exitCode: stats.exitCode,
    } : null,
    app: currentApp,
    runtime: runtimeReport(runtimeValue),
    error: errorToJson(err),
    browser: browserReport(),
    generatedAt: new Date().toISOString(),
    ...extra,
  };
}

function errDispSetReportStatus(text, error = false) {
  if (els.errDispReportStatus) {
    els.errDispReportStatus.textContent = text;
    els.errDispReportStatus.classList.toggle("error", error);
  }
}

async function errDispSend(report = lastErrDispReport) {
  if (!report) {
    return;
  }
  if (!errDispReportUrl) {
    errDispSetReportStatus("Report endpoint is not configured");
    return;
  }
  const body = JSON.stringify(report);
  if (body.length > ERR_DISP_MAX_REPORT_BYTES) {
    errDispSetReportStatus(`Report too large: ${body.length} bytes`, true);
    return;
  }
  errDispSetReportStatus("Reporting");
  try {
    const response = await fetch(errDispReportUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      keepalive: body.length <= 60 * 1024,
    });
    const text = await response.text();
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}${text ? `: ${text}` : ""}`);
    }
    const reply = parseJsonOrNull(text);
    errDispSetReportStatus(reply?.id ? `Reported: ${reply.id}` : "Reported");
  } catch (err) {
    errDispSetReportStatus(`Report failed: ${err.message}`, true);
    logWarn("err_disp report failed", err);
  }
}

function errDispShow(report) {
  lastErrDispReport = report;
  const json = JSON.stringify(report, null, 2);
  if (els.errDispTitle) {
    els.errDispTitle.textContent = errDispTitle(report);
  }
  if (els.errDispSummary) {
    els.errDispSummary.textContent = `${report.kind}: ${report.message}`;
  }
  if (els.errDispJson) {
    els.errDispJson.textContent = json;
  }
  if (els.errDispReportButton) {
    els.errDispReportButton.disabled = !errDispReportUrl;
  }
  errDispSetReportStatus(errDispReportUrl ? "Report ready" : "Report endpoint is not configured");
  if (els.errDispDialog?.showModal && !els.errDispDialog.open) {
    els.errDispDialog.showModal();
  }
  if (report.kind === "process_exit") {
    logInfo(ERR_DISP_FEATURE, report);
  } else {
    console.error(`${LOG_PREFIX} ${ERR_DISP_FEATURE}`, report);
  }
}

function normalizeArchivePath(name) {
  const parts = String(name).replaceAll("\\", "/").split("/").filter(Boolean);
  if (!parts.length || parts.some((part) => part === "." || part === ".." || part.includes(":"))) {
    return "";
  }
  return parts.join("/");
}

function guestPath(archivePath) {
  return `${MOUNT_ROOT}\\${archivePath.replaceAll("/", "\\")}`;
}

function syncPeImportPath(archivePath) {
  return /\.(dll|ocx)$/i.test(archivePath);
}

async function readArchive(file) {
  const reader = new zip.ZipReader(new zip.BlobReader(file));
  try {
    const entries = await reader.getEntries();
    const files = [];
    for (const entry of entries) {
      if (entry.directory) {
        continue;
      }
      const path = normalizeArchivePath(entry.filename);
      if (path) {
        files.push({ path, lower: path.toLowerCase(), entry });
      }
    }
    const exes = files.filter((file) => file.lower.endsWith(".exe"));
    return { reader, file, files, exes };
  } catch (err) {
    await reader.close().catch(() => {});
    throw err;
  }
}

function zipEntrySize(entry) {
  return Number(entry.uncompressedSize ?? entry.uncompressedSize64 ?? entry.size ?? 0);
}

async function extract(entry) {
  const blob = await entry.getData(new zip.BlobWriter());
  return new Uint8Array(await blob.arrayBuffer());
}

function readLe16(view, offset) {
  return view.getUint16(offset, true);
}

function readLe32(view, offset) {
  return view.getUint32(offset, true);
}

async function readBlobBytes(blob, offset, len) {
  return new Uint8Array(await blob.slice(offset, offset + len).arrayBuffer());
}

function findEndOfCentralDirectory(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let pos = bytes.length - 22; pos >= 0; pos -= 1) {
    if (readLe32(view, pos) === 0x06054b50) {
      return pos;
    }
  }
  return -1;
}

async function parseZipDirectory(blob) {
  const tailLen = Math.min(blob.size, 0xffff + 22);
  const tailOffset = blob.size - tailLen;
  const tail = await readBlobBytes(blob, tailOffset, tailLen);
  const eocd = findEndOfCentralDirectory(tail);
  if (eocd < 0) {
    throw new Error("ZIP central directory not found");
  }

  const tailView = new DataView(tail.buffer, tail.byteOffset, tail.byteLength);
  const totalEntries = readLe16(tailView, eocd + 10);
  const centralSize = readLe32(tailView, eocd + 12);
  const centralOffset = readLe32(tailView, eocd + 16);
  if (totalEntries === 0xffff || centralSize === 0xffff_ffff || centralOffset === 0xffff_ffff) {
    throw new Error("ZIP64 archives are not supported by the browser VFS yet");
  }

  const central = await readBlobBytes(blob, centralOffset, centralSize);
  const view = new DataView(central.buffer, central.byteOffset, central.byteLength);
  const entries = new Map();
  let allStored = true;
  let firstNonStored = "";
  let pos = 0;
  for (let index = 0; index < totalEntries; index += 1) {
    if (pos + 46 > central.length || readLe32(view, pos) !== 0x02014b50) {
      throw new Error("ZIP central directory is truncated");
    }
    const flags = readLe16(view, pos + 8);
    const method = readLe16(view, pos + 10);
    const compressedSize = readLe32(view, pos + 20);
    const uncompressedSize = readLe32(view, pos + 24);
    const nameLen = readLe16(view, pos + 28);
    const extraLen = readLe16(view, pos + 30);
    const commentLen = readLe16(view, pos + 32);
    const localOffset = readLe32(view, pos + 42);
    const nameStart = pos + 46;
    const nameEnd = nameStart + nameLen;
    if (nameEnd > central.length) {
      throw new Error("ZIP central directory filename is truncated");
    }
    const name = decoder.decode(central.subarray(nameStart, nameEnd));
    const normalized = normalizeArchivePath(name);
    if (method !== 0) {
      allStored = false;
      firstNonStored ||= normalized || name;
    }
    if (
      compressedSize === 0xffff_ffff ||
      uncompressedSize === 0xffff_ffff ||
      localOffset === 0xffff_ffff
    ) {
      throw new Error(`${normalized || name} uses ZIP64 metadata`);
    }
    if (normalized && method === 0) {
      const local = await readBlobBytes(blob, localOffset, 30);
      const localView = new DataView(local.buffer, local.byteOffset, local.byteLength);
      if (readLe32(localView, 0) !== 0x04034b50) {
        throw new Error(`${normalized} has an invalid local ZIP header`);
      }
      const localNameLen = readLe16(localView, 26);
      const localExtraLen = readLe16(localView, 28);
      entries.set(normalized.toLowerCase(), {
        offset: localOffset + 30 + localNameLen + localExtraLen,
        size: uncompressedSize,
        flags,
      });
    }
    pos = nameEnd + extraLen + commentLen;
  }
  return { entries, allStored, firstNonStored };
}

function idbRequest(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error || new Error("IndexedDB request failed"));
  });
}

async function openVfsDb() {
  if (typeof indexedDB === "undefined") {
    return null;
  }
  const request = indexedDB.open("wemu-vfs", 1);
  request.onupgradeneeded = () => {
    request.result.createObjectStore("files");
  };
  return idbRequest(request);
}

class ZipBaseVfs {
  constructor(files, reader, blob, storedEntries, keyOf) {
    this.entries = new Map();
    this.reader = reader;
    this.blob = blob;
    this.keyOf = keyOf;
    for (const file of files) {
      const guest = guestPath(file.path);
      const stored = storedEntries.get(file.path.toLowerCase());
      if (!stored) {
        throw new Error(`${file.path} is missing from the ZIP central directory`);
      }
      this.entries.set(this.keyOf(guest), {
        offset: stored.offset,
        size: stored.size || zipEntrySize(file.entry),
      });
    }
  }

  size(path) {
    return this.entries.get(this.keyOf(path))?.size ?? null;
  }

  async read(path, offset, len) {
    const key = this.keyOf(path);
    const item = this.entries.get(key);
    if (!item) {
      return null;
    }
    const start = Math.max(0, Math.min(item.size, offset));
    const end = Math.max(start, Math.min(item.size, offset + len));
    return readBlobBytes(this.blob, item.offset + start, end - start);
  }

  close() {
    this.reader?.close().catch(() => {});
    this.reader = null;
    this.blob = null;
  }
}

class MemoryOverlayVfs {
  constructor(keyOf) {
    this.files = new Map();
    this.keyOf = keyOf;
  }

  async read(path, offset, len) {
    const bytes = this.files.get(this.keyOf(path));
    if (!bytes) {
      return null;
    }
    return bytes.slice(offset, offset + len);
  }

  async write(path, offset, data, base) {
    const key = this.keyOf(path);
    let bytes = this.files.get(key);
    if (!bytes) {
      const baseSize = base.size(path);
      bytes = baseSize == null ? new Uint8Array(0) : await base.read(path, 0, baseSize);
      if (!bytes) {
        bytes = new Uint8Array(0);
      }
    }
    const end = offset + data.length;
    if (bytes.length < end) {
      const grown = new Uint8Array(end);
      grown.set(bytes);
      bytes = grown;
    } else {
      bytes = new Uint8Array(bytes);
    }
    bytes.set(data, offset);
    this.files.set(key, bytes);
    return data.length;
  }
}

class IndexedDbOverlayVfs {
  constructor(db, keyOf) {
    this.db = db;
    this.keyOf = keyOf;
  }

  async get(key) {
    const tx = this.db.transaction("files", "readonly");
    const value = await idbRequest(tx.objectStore("files").get(key));
    return value ? new Uint8Array(value) : null;
  }

  async put(key, bytes) {
    const tx = this.db.transaction("files", "readwrite");
    await idbRequest(tx.objectStore("files").put(bytes, key));
  }

  async read(path, offset, len) {
    const bytes = await this.get(this.keyOf(path));
    if (!bytes) {
      return null;
    }
    return bytes.slice(offset, offset + len);
  }

  async write(path, offset, data, base) {
    const key = this.keyOf(path);
    let bytes = await this.get(key);
    if (!bytes) {
      const baseSize = base.size(path);
      bytes = baseSize == null ? new Uint8Array(0) : await base.read(path, 0, baseSize);
      if (!bytes) {
        bytes = new Uint8Array(0);
      }
    }
    const end = offset + data.length;
    if (bytes.length < end) {
      const grown = new Uint8Array(end);
      grown.set(bytes);
      bytes = grown;
    } else {
      bytes = new Uint8Array(bytes);
    }
    bytes.set(data, offset);
    await this.put(key, bytes);
    return data.length;
  }

  close() {
    this.db.close();
  }
}

class LayeredVfs {
  constructor(base, overlay, keyOf) {
    this.base = base;
    this.overlay = overlay;
    this.keyOf = keyOf;
  }

  size(path) {
    return this.base.size(path);
  }

  async sizeForRegistration(path) {
    if (this.overlay.get) {
      const bytes = await this.overlay.get(this.keyOf(path));
      if (bytes) {
        return bytes.length;
      }
    }
    return this.base.size(path) ?? 0;
  }

  async read(path, offset, len) {
    const overlayBytes = await this.overlay.read(path, offset, len);
    if (overlayBytes) {
      return overlayBytes;
    }
    return this.base.read(path, offset, len);
  }

  async write(path, offset, data) {
    return this.overlay.write(path, offset, data, this.base);
  }

  close() {
    this.base.close();
    this.overlay.close?.();
  }
}

async function createArchiveVfs(files, reader, blob, storedEntries, keyOf) {
  const base = new ZipBaseVfs(files, reader, blob, storedEntries, keyOf);
  const db = await openVfsDb().catch(() => null);
  const overlay = db ? new IndexedDbOverlayVfs(db, keyOf) : new MemoryOverlayVfs(keyOf);
  return new LayeredVfs(base, overlay, keyOf);
}

function exeTitle(path) {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

function finishExeChoice(exe) {
  const resolve = exeChoiceResolve;
  exeChoiceResolve = null;
  if (els.exeDialog.open) {
    els.exeDialog.close();
  }
  if (resolve) {
    resolve(exe);
  }
}

function chooseExe(exes) {
  if (exes.length === 1) {
    return Promise.resolve(exes[0]);
  }
  setStatus("Choose program", `${exes.length} EXE files found`);
  els.exeChoices.replaceChildren();
  for (const exe of exes) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "exe-choice";
    const title = document.createElement("strong");
    title.textContent = exeTitle(exe.path);
    const path = document.createElement("span");
    path.textContent = exe.path;
    button.append(title, path);
    button.addEventListener("click", () => finishExeChoice(exe));
    els.exeChoices.append(button);
  }
  els.exeDialog.showModal();
  return new Promise((resolve) => {
    exeChoiceResolve = resolve;
  });
}

function clearRuntime() {
  running = false;
  stopRunLoop();
  currentApp = null;
  if (runtime) {
    logInfo("destroy runtime");
    runtime.destroy();
    runtime = null;
  }
  updateButtons();
}

async function mountArchive(wemu, archive, exe) {
  const directory = await parseZipDirectory(archive.file);
  logInfo("zip directory", {
    file: archive.file.name,
    bytes: archive.file.size,
    files: archive.files.length,
    allStored: directory.allStored,
    exe: exe.path,
  });
  if (!directory.allStored) {
    if (archive.file.size >= IN_MEMORY_ZIP_LIMIT) {
      throw new Error(
        `${directory.firstNonStored} is compressed; ZIPs 64 MB or larger must use storage mode`,
      );
    }
    logInfo("mount mode", { mode: "memory", reason: directory.firstNonStored });
    return mountArchiveInMemory(wemu, archive, exe);
  }

  logInfo("mount mode", { mode: "async-vfs" });
  const vfs = await createArchiveVfs(
    archive.files,
    archive.reader,
    archive.file,
    directory.entries,
    (path) => wemu.guestPathKey(path),
  );
  wemu.setVfs(vfs);
  wemu.enableAsyncVfsWrites();
  for (let index = 0; index < archive.files.length; index += 1) {
    const file = archive.files[index];
    const done = index + 1;
    setBusy(true, done / archive.files.length);
    setStatus("Mounting ZIP", `${done}/${archive.files.length}`);
    const guest = guestPath(file.path);
    wemu.addAsyncFile(guest, await vfs.sizeForRegistration(guest), false);
    if (syncPeImportPath(file.path)) {
      const bytes = await vfs.read(guest, 0, await vfs.sizeForRegistration(guest));
      if (bytes) {
        logInfo("preload pe import", { path: guest, bytes: bytes.length });
        wemu.addFile(guest, bytes);
      }
    }
  }
  const exeGuest = guestPath(exe.path);
  const exeBytes = await vfs.read(exeGuest, 0, await vfs.sizeForRegistration(exeGuest));
  if (!exeBytes) {
    throw new Error(`${exe.path} is missing from the ZIP VFS`);
  }
  return exeBytes;
}

async function mountArchiveInMemory(wemu, archive, exe) {
  let exeBytes = null;
  for (let index = 0; index < archive.files.length; index += 1) {
    const file = archive.files[index];
    const done = index + 1;
    setBusy(true, done / archive.files.length);
    setStatus("Extracting ZIP", `${done}/${archive.files.length}`);
    const bytes = await extract(file.entry);
    if (file.path === exe.path) {
      exeBytes = bytes;
    }
    wemu.addFile(guestPath(file.path), bytes);
  }
  if (!exeBytes) {
    throw new Error(`${exe.path} is missing from the ZIP`);
  }
  await archive.reader.close().catch(() => {});
  archive.reader = null;
  return exeBytes;
}

async function loadZipFile(file) {
  let archive = null;
  let wemu = null;
  clearRuntime();
  setBusy(true, 0);
  setStatus("Reading ZIP", file.name);
  logInfo("open zip", { name: file.name, bytes: file.size });

  try {
    archive = await readArchive(file);
    if (!archive.exes.length) {
      throw new Error("ZIP has no .exe files");
    }

    const exe = await chooseExe(archive.exes);
    if (!exe) {
      setStatus("Ready", "Load a ZIP to start");
      return;
    }

    setStatus("Starting emulator", exe.path);
    logInfo("selected exe", { path: exe.path });
    currentApp = {
      zip: { name: file.name, bytes: file.size },
      exe: { path: exe.path, guestPath: guestPath(exe.path) },
    };
    wemu = await Wemu.create();
    const exeBytes = await mountArchive(wemu, archive, exe);
    const runningExe = guestPath(exe.path);
    setStatus("Running", runningExe);
    wemu.loadExe(runningExe, exeBytes);
    logInfo("run start", { path: runningExe });
    runtime = wemu;
    wemu = null;
    archive = null;
    if (els.currentGame) {
      els.currentGame.textContent = exe.path;
    }
    running = true;
    renderFrame();
    refreshStats();
    nextFrameAt = 0;
    updateButtons();
    scheduleRun();
  } catch (err) {
    errDispShow(errDispReport("startup_error", err.message, wemu || runtime, err, {
      phase: "load_zip",
    }));
    if (wemu) {
      wemu.destroy();
    }
    setStatus("Could not start", err.message, true);
    logError("startup failed", err);
  } finally {
    if (archive?.reader) {
      await archive.reader.close().catch(() => {});
    }
    setBusy(false);
  }
}

function renderFrame() {
  if (!runtime) {
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, els.screen.width, els.screen.height);
    return;
  }
  const frame = runtime.frame();
  setCanvasModeSize(frame.width, frame.height);
  ctx.putImageData(frame.image, 0, 0);
}

function refreshStats() {
  const hasStatsDom =
    els.stopValue ||
    els.insnsValue ||
    els.eipValue ||
    els.eaxValue ||
    els.espValue ||
    els.guestOutput;
  if (!hasStatsDom) {
    return;
  }
  if (!runtime) {
    if (els.stopValue) {
      els.stopValue.textContent = "-";
    }
    if (els.insnsValue) {
      els.insnsValue.textContent = "0";
    }
    if (els.eipValue) {
      els.eipValue.textContent = "00000000";
    }
    if (els.eaxValue) {
      els.eaxValue.textContent = "00000000";
    }
    if (els.espValue) {
      els.espValue.textContent = "00000000";
    }
    if (els.guestOutput) {
      els.guestOutput.textContent = "";
    }
    return;
  }
  const stats = runtime.stats();
  if (els.stopValue) {
    els.stopValue.textContent = stopLabel(stats.stop, stats.exitCode);
  }
  if (els.insnsValue) {
    els.insnsValue.textContent = stats.insns;
  }
  if (els.eipValue) {
    els.eipValue.textContent = hex32(stats.eip);
  }
  if (els.eaxValue) {
    els.eaxValue.textContent = hex32(stats.eax);
  }
  if (els.espValue) {
    els.espValue.textContent = hex32(stats.esp);
  }
  if (els.guestOutput) {
    els.guestOutput.textContent = runtime.output();
  }
}

async function runChunk(now) {
  runFrame = 0;
  if (!runtime || !running) {
    return;
  }
  if (runFrameBusy) {
    scheduleRun();
    return;
  }
  if (nextFrameAt === 0) {
    nextFrameAt = now;
  }
  if (now < nextFrameAt) {
    scheduleRun();
    return;
  }
  const activeRuntime = runtime;
  runFrameBusy = true;
  try {
    const stop = await activeRuntime.runOneFrame();
    if (runtime !== activeRuntime) {
      return;
    }
    renderFrame();
    refreshStats();
    nextFrameAt += SCHEDULER_FRAME_INTERVAL_MS;
    if (nextFrameAt <= now) {
      nextFrameAt = now + SCHEDULER_FRAME_INTERVAL_MS;
    }
    if (stop !== 0) {
      running = false;
      renderFrame();
      refreshStats();
      const stats = runtime.stats();
      const label = stopLabel(stop, stats.exitCode);
      logInfo("run stop", {
        stop: label,
        insns: stats.insns,
        eip: hex32(stats.eip),
        esp: hex32(stats.esp),
      });
      errDispShow(errDispReport(errDispStopKind(stop), label, runtime));
      setStatus(label);
      updateButtons();
      return;
    }
  } catch (err) {
    if (runtime !== activeRuntime) {
      return;
    }
    running = false;
    errDispShow(errDispReport("runtime_error", err.message, activeRuntime, err));
    setStatus("Runtime stopped", err.message, true);
    updateButtons();
    logError("runtime stopped", err);
  } finally {
    runFrameBusy = false;
    if (runtime === activeRuntime && running) {
      scheduleRun();
    }
  }
}

function toggleRun() {
  if (!runtime) {
    return;
  }
  running = !running;
  logInfo(running ? "run resume" : "run pause");
  setStatus(running ? "Running" : "Paused");
  updateButtons();
  if (running) {
    nextFrameAt = 0;
    scheduleRun();
  }
}

function canvasPointFromClient(clientX, clientY) {
  const rect = els.screen.getBoundingClientRect();
  const width = runtime ? runtime.api.wemu_width(runtime.handle) : els.screen.width;
  const height = runtime ? runtime.api.wemu_height(runtime.handle) : els.screen.height;
  const x = Math.floor(((clientX - rect.left) / rect.width) * width);
  const y = Math.floor(((clientY - rect.top) / rect.height) * height);
  return {
    x: Math.max(0, Math.min(width - 1, x)),
    y: Math.max(0, Math.min(height - 1, y)),
  };
}

function canvasPoint(event) {
  return canvasPointFromClient(event.clientX, event.clientY);
}

function capturePointer(event) {
  if (els.screen.setPointerCapture && event.pointerId !== undefined) {
    els.screen.setPointerCapture(event.pointerId);
  }
}

function focusScreen() {
  try {
    els.screen.focus({ preventScroll: true });
  } catch {
    els.screen.focus();
  }
}

function pointerHandledByTouchEvents(event) {
  return useTouchEvents && event.pointerType === "touch";
}

function pointerButton(event) {
  if (event.isPrimary === false) {
    return null;
  }
  if (pointerHandledByTouchEvents(event)) {
    return null;
  }
  if (event.pointerType === "touch" || event.pointerType === "pen" || event.button === 0) {
    return "left";
  }
  if (event.button === 2) {
    return "right";
  }
  return null;
}

function downloadPng() {
  if (!runtime) {
    return;
  }
  const url = URL.createObjectURL(runtime.pngBlob());
  const link = document.createElement("a");
  link.href = url;
  link.download = "wemu.png";
  link.click();
  URL.revokeObjectURL(url);
}

function openZipPicker() {
  if (!wasmReady || busy) {
    return;
  }
  els.zipFile.value = "";
  els.zipFile.click();
}

els.loadButton?.addEventListener("click", openZipPicker);
els.idleLoadButton?.addEventListener("click", openZipPicker);
els.zipFile.addEventListener("change", () => {
  const file = els.zipFile.files && els.zipFile.files.length ? els.zipFile.files[0] : null;
  if (file) {
    loadZipFile(file);
  }
});
els.pauseButton?.addEventListener("click", toggleRun);
els.shotButton?.addEventListener("click", downloadPng);
els.errDispReportButton?.addEventListener("click", () => errDispSend());
els.errDispClose?.addEventListener("click", () => els.errDispDialog?.close());
els.exeCancel.addEventListener("click", () => finishExeChoice(null));
els.exeDialog.addEventListener("cancel", (event) => {
  event.preventDefault();
  finishExeChoice(null);
});
els.screen.addEventListener("contextmenu", (event) => event.preventDefault());
els.screen.addEventListener("pointermove", (event) => {
  if (!runtime || event.isPrimary === false || pointerHandledByTouchEvents(event)) {
    return;
  }
  const { x, y } = canvasPoint(event);
  runtime.mouseMove(x, y);
});
els.screen.addEventListener("pointerdown", (event) => {
  const button = pointerButton(event);
  if (!runtime || !button) {
    return;
  }
  event.preventDefault();
  focusScreen();
  capturePointer(event);
  pointerButtons.set(event.pointerId, button);
  const { x, y } = canvasPoint(event);
  runtime.mouseMove(x, y);
  if (button === "right") {
    runtime.mouseRightDown(x, y);
  } else {
    runtime.mouseDown(x, y);
  }
});
els.screen.addEventListener("pointerup", (event) => {
  const trackedButton = pointerButtons.get(event.pointerId);
  const button = pointerButton(event) || trackedButton;
  if (trackedButton) {
    pointerButtons.delete(event.pointerId);
  }
  if (!runtime || !button) {
    return;
  }
  event.preventDefault();
  const { x, y } = canvasPoint(event);
  runtime.mouseMove(x, y);
  if (button === "right") {
    runtime.mouseRightUp(x, y);
  } else {
    runtime.mouseUp(x, y);
  }
});
els.screen.addEventListener("pointercancel", (event) => {
  const button = pointerButtons.get(event.pointerId);
  pointerButtons.delete(event.pointerId);
  if (!runtime || !button) {
    return;
  }
  event.preventDefault();
  const { x, y } = canvasPoint(event);
  runtime.mouseMove(x, y);
  if (button === "right") {
    runtime.mouseRightUp(x, y);
  } else {
    runtime.mouseUp(x, y);
  }
});

if (useTouchEvents) {
  let activeTouchId = null;
  let activeTouchButton = null;
  let rightTouchIds = [];
  const touchOptions = { passive: false };
  const activeTouch = (event) => {
    if (activeTouchId === null) {
      return null;
    }
    for (const touch of event.changedTouches) {
      if (touch.identifier === activeTouchId) {
        return touch;
      }
    }
    for (const touch of event.touches) {
      if (touch.identifier === activeTouchId) {
        return touch;
      }
    }
    return null;
  };
  const touchById = (touches, id) => {
    for (const touch of touches) {
      if (touch.identifier === id) {
        return touch;
      }
    }
    return null;
  };
  const rightTouchPoint = (event) => {
    const first =
      touchById(event.touches, rightTouchIds[0]) ||
      touchById(event.changedTouches, rightTouchIds[0]) ||
      event.touches[0] ||
      event.changedTouches[0];
    const second =
      touchById(event.touches, rightTouchIds[1]) ||
      touchById(event.changedTouches, rightTouchIds[1]) ||
      event.touches[1] ||
      event.changedTouches[1] ||
      first;
    if (!first) {
      return null;
    }
    return canvasPointFromClient(
      (first.clientX + second.clientX) / 2,
      (first.clientY + second.clientY) / 2,
    );
  };
  const clearTouch = () => {
    activeTouchId = null;
    activeTouchButton = null;
    rightTouchIds = [];
  };
  const startRightTouch = (event) => {
    const point = rightTouchPoint(event);
    if (!runtime || !point) {
      return;
    }
    if (activeTouchButton === "left") {
      runtime.mouseMove(point.x, point.y);
      runtime.mouseUp(point.x, point.y);
    }
    if (activeTouchButton !== "right") {
      runtime.mouseMove(point.x, point.y);
      runtime.mouseRightDown(point.x, point.y);
    }
    activeTouchButton = "right";
    activeTouchId = event.touches[0]?.identifier ?? null;
    rightTouchIds = [
      event.touches[0]?.identifier,
      event.touches[1]?.identifier,
    ].filter((id) => id !== undefined);
  };
  els.screen.addEventListener("touchstart", (event) => {
    if (!runtime || event.changedTouches.length === 0) {
      return;
    }
    event.preventDefault();
    focusScreen();
    if (event.touches.length >= 2) {
      startRightTouch(event);
      return;
    }
    if (activeTouchId !== null) {
      return;
    }
    const touch = event.changedTouches[0];
    activeTouchId = touch.identifier;
    activeTouchButton = "left";
    const { x, y } = canvasPointFromClient(touch.clientX, touch.clientY);
    runtime.mouseMove(x, y);
    runtime.mouseDown(x, y);
  }, touchOptions);
  els.screen.addEventListener("touchmove", (event) => {
    if (!runtime) {
      return;
    }
    if (activeTouchButton === "right") {
      event.preventDefault();
      const point = rightTouchPoint(event);
      if (point) {
        runtime.mouseMove(point.x, point.y);
      }
      return;
    }
    const touch = activeTouch(event);
    if (!touch) {
      return;
    }
    event.preventDefault();
    const { x, y } = canvasPointFromClient(touch.clientX, touch.clientY);
    runtime.mouseMove(x, y);
  }, touchOptions);
  const endTouch = (event) => {
    if (!runtime) {
      clearTouch();
      return;
    }
    if (activeTouchButton === "right") {
      event.preventDefault();
      const point = rightTouchPoint(event);
      if (point) {
        runtime.mouseMove(point.x, point.y);
        runtime.mouseRightUp(point.x, point.y);
      }
      clearTouch();
      return;
    }
    const touch = activeTouch(event);
    if (!touch) {
      return;
    }
    event.preventDefault();
    const { x, y } = canvasPointFromClient(touch.clientX, touch.clientY);
    runtime.mouseMove(x, y);
    runtime.mouseUp(x, y);
    clearTouch();
  };
  els.screen.addEventListener("touchend", endTouch, touchOptions);
  els.screen.addEventListener("touchcancel", endTouch, touchOptions);
}
els.screen.addEventListener("keydown", (event) => {
  if (!runtime) {
    return;
  }
  const vk = eventKeyToVk(event.key);
  if (vk === null) {
    return;
  }
  event.preventDefault();
  if (!event.repeat) {
    runtime.keyDown(vk);
  }
  if (isTextKey(event)) {
    runtime.text(event.key);
  }
});
els.screen.addEventListener("keyup", (event) => {
  if (!runtime) {
    return;
  }
  const vk = eventKeyToVk(event.key);
  if (vk === null) {
    return;
  }
  event.preventDefault();
  runtime.keyUp(vk);
});

window.addEventListener("beforeunload", () => {
  if (runtime) {
    runtime.destroy();
  }
});


renderFrame();
refreshStats();
updateButtons();
Wemu.preload((text) => setStatus(text, "Please wait"))
  .then(() => {
    wasmReady = true;
    setBusy(false);
    setStatus("Ready", "Load a ZIP to start");
  })
  .catch((err) => {
    setBusy(false);
    errDispShow(errDispReport("wasm_load_error", err.message, runtime, err));
    setStatus("Emulator failed to load", err.message, true);
    logError("emulator failed to load", err);
  });
