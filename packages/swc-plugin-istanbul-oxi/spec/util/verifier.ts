import { transformSync } from "@swc/core";
import * as path from "path";
import { assert } from "chai";
import { readInitialCoverage } from "./read-coverage";
import { EOL } from "os";
import { FileCoverageInterop } from "../../../istanbul-oxi-instrument-wasm/pkg";

const clone: typeof import("lodash.clone") = require("lodash.clone");

const pluginBinary = path.resolve(
  __dirname,
  "../../../../target/wasm32-wasi/debug/swc_plugin_istanbul_oxi.wasm"
);

/// Mimic instrumenter.
const instrumentSync = (
  code: string,
  filename: string,
  inputSourceMap?: unknown,
  instrumentOptions?: Record<string, any>
) => {
  const ret = transformSync(code, {
    filename: filename ?? "unknown",
    jsc: {
      parser: {
        syntax: "ecmascript",
        jsx: false,
      },
      target: "es2022",
      experimental: {
        plugins: [[pluginBinary, instrumentOptions ?? {}]],
      },
      preserveAllComments: true,
    },
  });

  return ret;
};

/**
 * Poorman's substitution for instrumenter::lastFileCoverage to get the coverage from instrumented codes.
 * SWC's plugin transform does not allow to pass arbiatary data other than transformed AST, using trailing comment
 * to grab out data from plugin.
 */
const lastFileCoverage = (code?: string) => {
  const lines = (code ?? "").split(EOL);
  const commentLine = lines
    .find((v) => v.includes("__coverage_data_json_comment__::"))
    ?.split("__coverage_data_json_comment__::")[1];

  const data = commentLine?.substring(0, commentLine.lastIndexOf("*/"));
  return data ? JSON.parse(data) : {};
};

type UnknownReserved = any;

class Verifier {
  private result: UnknownReserved;

  constructor(result: UnknownReserved) {
    this.result = result;
  }

  async verify(args, expectedOutput, expectedCoverage) {
    assert.ok(!this.result.err, (this.result.err || {}).message);

    getGlobalObject()[this.result.coverageVariable] = clone(
      this.result.baseline
    );
    const actualOutput = await this.result.fn(args);
    const cov = this.getFileCoverage();

    assert.ok(
      cov && typeof cov === "object",
      "No coverage found for [" + this.result.file + "]"
    );
    assert.deepEqual(actualOutput, expectedOutput, "Output mismatch");
    assert.deepEqual(
      Object.fromEntries(cov.getLineCoverage()),
      expectedCoverage.lines || {},
      "Line coverage mismatch"
    );
    assert.deepEqual(
      cov.f,
      expectedCoverage.functions || {},
      "Function coverage mismatch"
    );
    assert.deepEqual(
      cov.b,
      expectedCoverage.branches || {},
      "Branch coverage mismatch"
    );
    assert.deepEqual(
      cov.bT || {},
      expectedCoverage.branchesTrue || {},
      "Branch truthiness coverage mismatch"
    );
    assert.deepEqual(
      cov.s,
      expectedCoverage.statements || {},
      "Statement coverage mismatch"
    );
    assert.deepEqual(
      cov.data.inputSourceMap,
      expectedCoverage.inputSourceMap || undefined,
      "Input source map mismatch"
    );

    const initial = readInitialCoverage(this.getGeneratedCode());
    assert.ok(initial);
    assert.deepEqual(initial.coverageData, this.result.emptyCoverage);
    assert.ok(initial.path);
    if (this.result.file) {
      assert.equal(initial.path, this.result.file);
    }
    assert.equal(initial.gcv, this.result.coverageVariable);
    assert.ok(initial.hash);
  }

  getCoverage() {
    return getGlobalObject()[this.result.coverageVariable];
  }

  getFileCoverage() {
    const cov = this.getCoverage();

    const { _coverageSchema, hash, ...fileCoverage } = cov[Object.keys(cov)[0]];
    return new FileCoverageInterop(fileCoverage);
  }

  getGeneratedCode() {
    return this.result.generatedCode;
  }

  compileError() {
    return this.result.err;
  }
}

function extractTestOption(options, name, defaultValue) {
  let v = defaultValue;
  if (Object.prototype.hasOwnProperty.call(options, name)) {
    v = options[name];
  }
  return v;
}

const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;

function pad(str, len) {
  const blanks = "                                             ";
  if (str.length >= len) {
    return str;
  }
  return blanks.substring(0, len - str.length) + str;
}

function annotatedCode(code) {
  const codeArray = code.split("\n");
  let line = 0;
  const annotated = codeArray.map((str) => {
    line += 1;
    return pad(line, 6) + ": " + str;
  });
  return annotated.join("\n");
}

function getGlobalObject() {
  return new Function("return this")();
}

const create = (code, options = {}, instrumentOptions = {}, inputSourceMap) => {
  instrumentOptions.coverageVariable =
    instrumentOptions.coverageVariable || "__testing_coverage__";

  const debug = extractTestOption(options, "debug", process.env.DEBUG === "1");
  const file = extractTestOption(options, "file", __filename);
  const generateOnly = extractTestOption(options, "generateOnly", false);
  const noCoverage = extractTestOption(options, "noCoverage", false);
  const quiet = extractTestOption(options, "quiet", false);
  const coverageVariable = instrumentOptions.coverageVariable;
  const g = getGlobalObject();

  let instrumenterOutput;
  let wrapped;
  let fn;
  let verror;

  if (debug) {
    instrumentOptions.compact = false;
  }

  try {
    let out = instrumentSync(code, file, inputSourceMap, instrumentOptions);
    instrumenterOutput = out.code;

    if (debug) {
      console.log(
        "================== Original ============================================"
      );
      console.log(annotatedCode(code));
      console.log(
        "================== Generated ==========================================="
      );
      console.log(instrumenterOutput);
      console.log(
        "========================================================================"
      );
    }
  } catch (ex) {
    if (!quiet) {
      console.error(ex.stack);
    }
    verror = new Error(
      "Error instrumenting:\n" + annotatedCode(String(code)) + "\n" + ex.message
    );
  }
  if (!(verror || generateOnly)) {
    wrapped = "{ var output;\n" + instrumenterOutput + "\nreturn output;\n}";
    g[coverageVariable] = undefined;
    try {
      if (options.isAsync) {
        fn = new AsyncFunction("args", wrapped);
      } else {
        fn = new Function("args", wrapped);
      }
    } catch (ex) {
      console.error(ex.stack);
      verror = new Error(
        "Error compiling\n" + annotatedCode(code) + "\n" + ex.message
      );
    }
  }
  if (generateOnly || noCoverage) {
    if (verror) {
      throw verror;
    }
  }
  return new Verifier({
    err: verror,
    debug,
    file,
    fn,
    code,
    generatedCode: instrumenterOutput,
    coverageVariable,
    baseline: clone(g[coverageVariable]),
    emptyCoverage: lastFileCoverage(instrumenterOutput), //mimic instrumenter.getLastFileCoverage()
  });
};

export { create };
