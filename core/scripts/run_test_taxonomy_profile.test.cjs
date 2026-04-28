const test = require("node:test");
const assert = require("node:assert/strict");

const {
  parseArgs,
  runCommands,
} = require("./run_test_taxonomy_profile.cjs");

function silentStream() {
  let output = "";
  return {
    stream: {
      write(chunk) {
        output += chunk;
      },
    },
    output() {
      return output;
    },
  };
}

test("taxonomy profile parser accepts keep-going run mode", () => {
  const args = parseArgs(["--profile", "release-contracts", "--run", "--keep-going"]);
  assert.equal(args.profile, "release-contracts");
  assert.equal(args.run, true);
  assert.equal(args.keepGoing, true);
});

test("taxonomy profile runner stops at the first failing command by default", () => {
  const calls = [];
  const status = runCommands(["first", "second"], {
    spawnSyncImpl(_command, args) {
      calls.push(args[1]);
      return { status: calls.length === 1 ? 7 : 0 };
    },
    stderr: silentStream().stream,
  });

  assert.equal(status, 7);
  assert.deepEqual(calls, ["first"]);
});

test("taxonomy profile runner reports all command failures in keep-going mode", () => {
  const calls = [];
  const stderr = silentStream();
  const status = runCommands(["first", "second", "third"], {
    keepGoing: true,
    spawnSyncImpl(_command, args) {
      calls.push(args[1]);
      return { status: args[1] === "second" ? 0 : calls.length + 1 };
    },
    stderr: stderr.stream,
  });

  assert.equal(status, 2);
  assert.deepEqual(calls, ["first", "second", "third"]);
  assert.match(stderr.output(), /Taxonomy profile failures/);
  assert.match(stderr.output(), /1\/3 status=2: first/);
  assert.match(stderr.output(), /3\/3 status=4: third/);
});

test("taxonomy profile runner continues after spawn errors in keep-going mode", () => {
  const calls = [];
  const stderr = silentStream();
  const status = runCommands(["first", "second"], {
    keepGoing: true,
    spawnSyncImpl(_command, args) {
      calls.push(args[1]);
      if (args[1] === "first") {
        return { error: new Error("spawn failed"), status: null };
      }
      return { status: 0 };
    },
    stderr: stderr.stream,
  });

  assert.equal(status, 1);
  assert.deepEqual(calls, ["first", "second"]);
  assert.match(stderr.output(), /failed to start: spawn failed/);
});
