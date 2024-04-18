import fs from "fs";
import { WASI } from "@runno/wasi";

const wasi = new WASI({
  args: [],
  env: { SOME_KEY: "some value" },
  stdout: (out) => console.log("stdout", out),
  stderr: (err) => console.error("stderr", err),
  stdin: () => prompt("stdin:"),
  fs: {
    "/some-file.txt": {
      path: "/some-file.txt",
      timestamps: {
        access: new Date(),
        change: new Date(),
        modification: new Date(),
      },
      mode: "string",
      content: "Some content for the file.",
    },
  },
});

const filename = process.argv[2];
const module = new WebAssembly.Module(fs.readFileSync(filename));
const instance = new WebAssembly.Instance(module, wasi.getImportObject());
const result = {module, instance};
wasi.start(result);
