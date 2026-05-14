#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "..");
const defaultWasm = path.join(repo, "target/wasm32-unknown-unknown/release/wemu.wasm");

function usage() {
  console.error(
    [
      "usage: node tools/wemu-node.mjs --exe HOST_EXE [--guest-exe C:\\\\game.exe]",
      "       [--wasm target/wasm32-unknown-unknown/release/wemu.wasm]",
      "       [--mount C=/host/dir] [--file C:\\\\path=host/file]",
      "       [--async-vfs] [--max-insns N] [--screenshot out.png]",
    ].join("\n"),
  );
}

function parseArgs(argv) {
  const cfg = {
    wasm: defaultWasm,
    exe: "",
    guestExe: "",
    mounts: [],
    files: [],
    asyncVfs: false,
    maxInsns: 1_000_000,
    chunkInsns: 1_000_000,
    screenshot: "/tmp/wemu-wasm.png",
    width: 640,
    height: 480,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const next = () => {
      if (i + 1 >= argv.length) {
        throw new Error(`${arg} needs a value`);
      }
      return argv[++i];
    };
    switch (arg) {
      case "--help":
      case "-h":
        usage();
        process.exit(0);
        break;
      case "--wasm":
        cfg.wasm = next();
        break;
      case "--exe":
        cfg.exe = next();
        break;
      case "--guest-exe":
        cfg.guestExe = next();
        break;
      case "--mount":
        cfg.mounts.push(parseMount(next()));
        break;
      case "--file":
        cfg.files.push(parseFile(next()));
        break;
      case "--async-vfs":
        cfg.asyncVfs = true;
        break;
      case "--max-insns":
        cfg.maxInsns = parseCount(next(), arg);
        break;
      case "--chunk-insns":
        cfg.chunkInsns = parseCount(next(), arg);
        break;
      case "--screenshot":
        cfg.screenshot = next();
        break;
      case "--width":
        cfg.width = parseCount(next(), arg);
        break;
      case "--height":
        cfg.height = parseCount(next(), arg);
        break;
      default:
        throw new Error(`unknown argument ${arg}`);
    }
  }
  if (!cfg.exe) {
    throw new Error("--exe is required");
  }
  if (cfg.chunkInsns === 0) {
    throw new Error("--chunk-insns must be greater than zero");
  }
  return cfg;
}

function parseCount(value, name) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    throw new Error(`invalid ${name}: ${value}`);
  }
  return parsed;
}

function parseMount(spec) {
  const at = spec.indexOf("=");
  if (at < 1) {
    throw new Error(`mount must look like C=/dir, got ${spec}`);
  }
  const drive = spec.slice(0, at).toUpperCase();
  if (!/^[A-Z]$/.test(drive)) {
    throw new Error(`invalid drive in mount ${spec}`);
  }
  return { drive, root: path.resolve(spec.slice(at + 1)) };
}

function parseFile(spec) {
  const at = spec.indexOf("=");
  if (at < 1) {
    throw new Error(`file must look like C:\\path=host/file, got ${spec}`);
  }
  return { guest: spec.slice(0, at), host: path.resolve(spec.slice(at + 1)) };
}

function guestPathForMount(mount, hostPath) {
  const rel = path.relative(mount.root, hostPath);
  if (rel.startsWith("..") || path.isAbsolute(rel)) {
    return null;
  }
  const guestRel = rel.split(path.sep).filter(Boolean).join("\\");
  return `${mount.drive}:\\${guestRel}`;
}

function vfsKey(guest) {
  return String(guest).replaceAll("/", "\\").toLowerCase();
}

function syncPeImportPath(guest) {
  return /\.(dll|ocx)$/i.test(guest);
}

function inferGuestExe(cfg) {
  if (cfg.guestExe) {
    return cfg.guestExe;
  }
  const exe = path.resolve(cfg.exe);
  for (const mount of cfg.mounts) {
    const guest = guestPathForMount(mount, exe);
    if (guest) {
      return guest;
    }
  }
  return `C:\\${path.basename(exe)}`;
}

function listFiles(root) {
  const out = [];
  const pending = [root];
  while (pending.length) {
    const current = pending.pop();
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        pending.push(full);
      } else if (entry.isFile()) {
        out.push(full);
      }
    }
  }
  return out;
}

function stopName(code) {
  return {
    0: "Running",
    1: "ExitProcess",
    2: "MaxInstructions",
    3: "Breakpoint",
    4: "HleBooted",
    5: "CpuHalted",
    6: "FrontendQuit",
    7: "Waiting",
  }[code] ?? `Unknown(${code})`;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

async function main() {
  const cfg = parseArgs(process.argv.slice(2));
  const wasmBytes = fs.readFileSync(cfg.wasm);
  let e = null;
  const imports = {
    env: {
      wemu_console_log(ptr, len) {
        if (e) {
          console.log(decoder.decode(new Uint8Array(e.memory.buffer, ptr, len)));
        }
      },
      wemu_now_ms() {
        return performance.now();
      },
      wemu_canvas_text() {
        return 0;
      },
    },
  };
  const { instance } = await WebAssembly.instantiate(wasmBytes, imports);
  e = instance.exports;
  const handle = e.wemu_new(cfg.width, cfg.height);
  if (!handle) {
    throw new Error("wemu_new failed");
  }

  const readUtf8 = (ptr, len) => decoder.decode(new Uint8Array(e.memory.buffer, ptr, len));
  const lastError = () => readUtf8(e.wemu_last_error_ptr(handle), e.wemu_last_error_len(handle));
  const check = (rc, label) => {
    if (rc < 0) {
      throw new Error(`${label}: ${lastError()}`);
    }
    return rc;
  };
  const withBytes = (bytes, fn) => {
    const ptr = e.wemu_alloc(bytes.length);
    if (!ptr && bytes.length) {
      throw new Error(`wemu_alloc(${bytes.length}) failed`);
    }
    new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
    try {
      return fn(ptr, bytes.length);
    } finally {
      e.wemu_free(ptr, bytes.length);
    }
  };
  const withString = (text, fn) => withBytes(encoder.encode(text), fn);
  const addFile = (guest, host) => {
    const data = fs.readFileSync(host);
    withString(guest, (pathPtr, pathLen) => {
      withBytes(data, (dataPtr, dataLen) => {
        check(e.wemu_add_file(handle, pathPtr, pathLen, dataPtr, dataLen), `add ${guest}`);
      });
    });
  };
  const vfsFiles = new Map();
  const vfsOverlay = new Map();
  const addAsyncFile = (guest, host) => {
    const size = fs.statSync(host).size;
    const lo = size >>> 0;
    const hi = Math.floor(size / 0x100000000) >>> 0;
    withString(guest, (pathPtr, pathLen) => {
      check(
        e.wemu_add_async_file(handle, pathPtr, pathLen, lo, hi, 0),
        `add async ${guest}`,
      );
    });
    vfsFiles.set(vfsKey(guest), host);
  };
  const completeVfs = (id, status, transferred, data = new Uint8Array(0)) => {
    withBytes(data, (ptr, len) => {
      check(
        e.wemu_complete_vfs_request(handle, id, status, transferred, ptr, len),
        `complete VFS ${id}`,
      );
    });
  };
  const serviceVfsRequest = () => {
    const id = e.wemu_pending_vfs_request_id(handle);
    const kind = e.wemu_pending_vfs_request_kind(handle);
    if (!id || !kind) {
      return;
    }
    const guest = readUtf8(
      e.wemu_pending_vfs_request_path_ptr(handle),
      e.wemu_pending_vfs_request_path_len(handle),
    );
    const key = vfsKey(guest);
    const offset =
      e.wemu_pending_vfs_request_offset_lo(handle) +
      e.wemu_pending_vfs_request_offset_hi(handle) * 0x100000000;
    const len = e.wemu_pending_vfs_request_len(handle);
    if (kind === 1) {
      const overlay = vfsOverlay.get(key);
      const source = overlay || (vfsFiles.has(key) ? fs.readFileSync(vfsFiles.get(key)) : null);
      if (!source) {
        completeVfs(id, 2, 0);
        return;
      }
      const bytes = new Uint8Array(source).slice(offset, offset + len);
      completeVfs(id, 0, bytes.length, bytes);
      return;
    }
    if (kind === 2) {
      const dataPtr = e.wemu_pending_vfs_request_data_ptr(handle);
      const dataLen = e.wemu_pending_vfs_request_data_len(handle);
      const data = dataPtr && dataLen
        ? new Uint8Array(new Uint8Array(e.memory.buffer, dataPtr, dataLen))
        : new Uint8Array(0);
      let bytes = vfsOverlay.get(key);
      if (!bytes) {
        bytes = vfsFiles.has(key)
          ? new Uint8Array(fs.readFileSync(vfsFiles.get(key)))
          : new Uint8Array(0);
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
      vfsOverlay.set(key, bytes);
      completeVfs(id, 0, data.length);
      return;
    }
    completeVfs(id, 5, 0);
  };

  if (cfg.asyncVfs) {
    check(e.wemu_enable_async_vfs_writes(handle), "enable async VFS writes");
  }

  let mounted = 0;
  for (const mount of cfg.mounts) {
    for (const host of listFiles(mount.root)) {
      const guest = guestPathForMount(mount, host);
      if (cfg.asyncVfs) {
        addAsyncFile(guest, host);
        if (syncPeImportPath(guest)) {
          addFile(guest, host);
        }
      } else {
        addFile(guest, host);
      }
      mounted++;
    }
  }
  for (const file of cfg.files) {
    if (cfg.asyncVfs) {
      addAsyncFile(file.guest, file.host);
      if (syncPeImportPath(file.guest)) {
        addFile(file.guest, file.host);
      }
    } else {
      addFile(file.guest, file.host);
    }
    mounted++;
  }

  const guestExe = inferGuestExe(cfg);
  const exeBytes = fs.readFileSync(cfg.exe);
  withString(guestExe, (pathPtr, pathLen) => {
    withBytes(exeBytes, (exePtr, exeLen) => {
      check(e.wemu_load_exe(handle, pathPtr, pathLen, exePtr, exeLen), "load exe");
    });
  });

  const currentInsns = () => e.wemu_insns_lo(handle) + e.wemu_insns_hi(handle) * 0x1_0000_0000;
  let stop = 0;
  while (currentInsns() < cfg.maxInsns) {
    stop = check(e.wemu_run_one_frame(handle, 0, 0), "run frame");
    serviceVfsRequest();
    if (stop !== 0) {
      break;
    }
  }

  check(e.wemu_screenshot_png(handle), "screenshot");
  const pngPtr = e.wemu_blob_ptr(handle);
  const pngLen = e.wemu_blob_len(handle);
  fs.writeFileSync(cfg.screenshot, new Uint8Array(e.memory.buffer, pngPtr, pngLen));

  const insns = currentInsns();
  const exitCode = e.wemu_exit_code(handle);
  console.log(
    `wemu wasm stopped=${stopName(stop)} exit_code=${exitCode} insns=${insns} eip=${e
      .wemu_eip(handle)
      .toString(16)
      .padStart(8, "0")} mounted_files=${mounted} screenshot=${cfg.screenshot}`,
  );
  e.wemu_destroy(handle);
}

main().catch((err) => {
  console.error(err.message);
  process.exit(1);
});
