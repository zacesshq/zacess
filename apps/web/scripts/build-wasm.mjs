// Builds the core for the browser: cargo compiles zacess-wasm to WebAssembly, wasm-bindgen writes
// the JavaScript glue next to it and wasm-opt shrinks it when installed. The output lands in
// src/core/pkg, which is generated and never committed.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../..");
const out = resolve(here, "../src/core/pkg");
const profile = "wasm-release";
const target = "wasm32-unknown-unknown";

function run(command, args, options = {}) {
  console.log(`> ${command} ${args.join(" ")}`);
  execFileSync(command, args, { stdio: "inherit", cwd: root, ...options });
}

function found(command) {
  try {
    execFileSync(command, ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

run(process.env.CARGO ?? "cargo", ["build", "-p", "zacess-wasm", "--target", target, "--profile", profile]);

const wasm = join(root, "target", target, profile, "zacess_wasm.wasm");
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
run(process.env.WASM_BINDGEN ?? "wasm-bindgen", ["--target", "web", "--out-dir", out, wasm]);

const bindgenOutput = join(out, "zacess_wasm_bg.wasm");
if (found("wasm-opt")) {
  // the features rustc enables by default for this target, which wasm-opt must be told about
  const features = [
    "--enable-bulk-memory",
    "--enable-sign-ext",
    "--enable-mutable-globals",
    "--enable-nontrapping-float-to-int",
    "--enable-reference-types",
    "--enable-multivalue",
  ];
  run("wasm-opt", ["-Oz", ...features, bindgenOutput, "-o", bindgenOutput]);
} else {
  console.log("wasm-opt was not found, so the core is left unshrunk");
}

if (!existsSync(bindgenOutput)) throw new Error("wasm-bindgen wrote no module");
console.log(`core: ${(statSync(bindgenOutput).size / 1024).toFixed(0)} KiB`);
