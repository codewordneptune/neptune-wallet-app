import { amount_to_positive_fixed } from "@/utils/math-util";
import assert from "node:assert";
import test from "node:test";

test("Sane number formatting", () => {
  assert.strictEqual(amount_to_positive_fixed("1"), "1.0000");
  assert.strictEqual(amount_to_positive_fixed("-5.5"), "5.5000");
  assert.strictEqual(amount_to_positive_fixed("10.12349"), "10.1234");
  assert.strictEqual(amount_to_positive_fixed(""), "0");
});
