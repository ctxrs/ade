#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const {
  buildStorageClientFromEnv,
  readFileBytes,
} = require("./lib/release_storage.cjs");

function parseArgs(argv) {
  const options = {
    allowMissing: false,
    command: "",
    contentType: "application/octet-stream",
    fileSizeLimitBytes: "5368709120",
    objectPath: "",
    outPath: "",
    srcPath: "",
    upsert: false,
    verifyExisting: false,
  };
  const [command, ...rest] = argv;
  options.command = command || "";
  for (let index = 0; index < rest.length; index += 1) {
    const arg = rest[index];
    if (arg === "--object-path") {
      options.objectPath = rest[++index] || "";
      continue;
    }
    if (arg === "--src") {
      options.srcPath = rest[++index] || "";
      continue;
    }
    if (arg === "--out") {
      options.outPath = rest[++index] || "";
      continue;
    }
    if (arg === "--content-type") {
      options.contentType = rest[++index] || "";
      continue;
    }
    if (arg === "--upsert") {
      const raw = String(rest[++index] || "").trim();
      if (raw !== "true" && raw !== "false") {
        throw new Error("--upsert expects true or false");
      }
      options.upsert = raw === "true";
      continue;
    }
    if (arg === "--verify-existing") {
      options.verifyExisting = true;
      continue;
    }
    if (arg === "--allow-missing") {
      options.allowMissing = true;
      continue;
    }
    if (arg === "--file-size-limit-bytes") {
      options.fileSizeLimitBytes = rest[++index] || "";
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }
  return options;
}

function printUsage() {
  console.log(`usage: node core/scripts/release_storage_object.cjs <command> [options]

commands:
  put             Upload --src to --object-path
  get             Download --object-path to --out
  delete          Delete --object-path
  ensure-bucket   Ensure the R2 bucket is reachable
  public-url      Print the public URL for --object-path

common env:
  RELEASE_STORAGE_PROVIDER=r2
  RELEASE_STORAGE_BUCKET=<bucket> (fallback: CTX_RELEASES_R2_BUCKET, CTX_RELEASE_R2_BUCKET, or RELEASE_R2_BUCKET)

R2 env:
  RELEASE_R2_ENDPOINT or RELEASE_R2_ACCOUNT_ID
  RELEASE_R2_ACCESS_KEY_ID
  RELEASE_R2_SECRET_ACCESS_KEY
  RELEASE_R2_REGION (default: auto)
`);
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help || !options.command) {
    printUsage();
    return;
  }
  const client = buildStorageClientFromEnv(process.env);
  if (options.command === "put") {
    if (!options.objectPath || !options.srcPath) {
      throw new Error("put requires --object-path and --src");
    }
    await client.putObject({
      body: readFileBytes(options.srcPath),
      contentType: options.contentType,
      objectPath: options.objectPath,
      upsert: options.upsert,
      verifyExisting: options.verifyExisting,
    });
    return;
  }
  if (options.command === "get") {
    if (!options.objectPath || !options.outPath) {
      throw new Error("get requires --object-path and --out");
    }
    const bytes = await client.getObjectBuffer(options.objectPath, {
      allowMissing: options.allowMissing,
    });
    if (bytes === null) {
      return;
    }
    fs.mkdirSync(path.dirname(path.resolve(options.outPath)), { recursive: true });
    fs.writeFileSync(options.outPath, bytes);
    return;
  }
  if (options.command === "delete") {
    if (!options.objectPath) {
      throw new Error("delete requires --object-path");
    }
    await client.deleteObject(options.objectPath);
    return;
  }
  if (options.command === "ensure-bucket") {
    await client.ensureBucket({ fileSizeLimitBytes: options.fileSizeLimitBytes });
    return;
  }
  if (options.command === "public-url") {
    if (!options.objectPath) {
      throw new Error("public-url requires --object-path");
    }
    process.stdout.write(`${client.publicObjectUrl(options.objectPath)}\n`);
    return;
  }
  throw new Error(`unsupported command: ${options.command}`);
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error?.stack || error?.message || String(error));
    process.exit(1);
  });
}

module.exports = {
  parseArgs,
};
