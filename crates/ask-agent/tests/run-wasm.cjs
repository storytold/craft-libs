const fs = require("node:fs");

async function main() {
  if (process.argv.length < 3) throw new Error("Pass the compiled wasm_poll test binary");
  for (const path of process.argv.slice(2)) {
    const { instance } = await WebAssembly.instantiate(fs.readFileSync(path), {});
    if (typeof instance.exports.main !== "function") throw new Error("Test main export missing");
    instance.exports.main();
    console.log(`WASM polling regression passed: ${path}`);
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
