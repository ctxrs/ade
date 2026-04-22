import assert from "node:assert/strict";
import test from "node:test";

import {
  findRuntimeViolations,
  isProductionRustFile,
} from "./check_rust_runtime_panic_traps.mjs";

test("panic-trap production file filter excludes test-only source trees", () => {
  assert.equal(isProductionRustFile("crates/ctx-http/src/lib.rs"), true);
  assert.equal(isProductionRustFile("crates/ctx-http/src/lib_tests/session_artifacts.rs"), false);
  assert.equal(isProductionRustFile("crates/ctx-http/src/api/ws/tests/http.rs"), false);
  assert.equal(isProductionRustFile("crates/ctx-providers/src/crp/normalize/tests/mod.rs"), false);
  assert.equal(isProductionRustFile("crates/ctx-http/tests/external.rs"), false);
});

test("panic-trap runtime scan ignores cfg(test) blocks but flags runtime unwraps", () => {
  const violations = findRuntimeViolations(`
fn runtime_path() {
  value.unwrap();
}

#[cfg(test)]
mod tests {
  fn helper() {
    test_value.unwrap();
  }
}
`);

  assert.deepEqual(
    violations.map((violation) => violation.line),
    [3],
  );
});
