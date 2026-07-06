import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, test } from "bun:test";

const nativeAddonPath = fileURLToPath(new URL("../index.node", import.meta.url));
const dataDir = fileURLToPath(new URL("../../data/range-strata", import.meta.url));

let PokerHandsRange;

beforeAll(async () => {
  if (!existsSync(nativeAddonPath)) {
    throw new Error(
      "Missing range-store-native/index.node. Run `bun run build:native` in range-store-native before `bun run test:sdk`.",
    );
  }
  if (!existsSync(dataDir)) {
    throw new Error(`Missing SDK contract fixture data: ${dataDir}`);
  }
  ({ PokerHandsRange } = await import("../index.js"));
});

function openStore() {
  return new PokerHandsRange({
    dataDir,
    maxOpenHandles: 2,
    verifyChecksums: false,
  });
}

function baseDimension() {
  return {
    strategy: "default",
    playerCount: 6,
    depthBb: 100,
  };
}

describe("PokerHandsRange SDK contract", () => {
  test("keeps constructor lightweight and lazily warms schema cache on strategy query", () => {
    const store = openStore();

    expect(store.stats()).toMatchObject({
      code: 0,
      data: {
        openHandleCount: 0,
        schemaCount: 0,
      },
      message: null,
    });

    const line = store.getConcreteLines({
      ...baseDimension(),
      concreteLine: "F-F-F",
    });
    expect(line.code).toBe(0);
    expect(line.data.lines).toHaveLength(1);
    expect(line.data.lines[0].concreteLineId).toBeGreaterThan(0);
    expect(store.stats().data.schemaCount).toBe(0);

    const result = store.queryBatch({
      ...baseDimension(),
      items: [
        { concreteLineId: 1, holeCards: "AA" },
        { concreteLineId: 1, holeCards: "AsXx" },
      ],
    });

    expect(result).toMatchObject({
      code: 0,
      message: null,
    });
    expect(result.data.results[0].actions.length).toBeGreaterThan(0);
    expect(result.data.results[0].handCode).toBe("AA");
    expect(result.data.results[0].error).toBeUndefined();
    expect(result.data.results[1].error).toMatchObject({
      code: 1000,
    });
    expect(result.data.results[1].error.message).toContain("Invalid card format");
    expect(store.stats().data.schemaCount).toBeGreaterThan(0);
  });

  test("returns per-item 404 errors without converting them to 500", () => {
    const store = openStore();

    const result = store.queryBatch({
      ...baseDimension(),
      items: [{ concreteLineId: 999999999, holeCards: "AA" }],
    });

    expect(result.code).toBe(0);
    expect(result.message).toBeNull();
    expect(result.data.results[0].error).toMatchObject({
      code: 404,
    });
    expect(result.data.results[0].error.message).toContain("concrete_line_id=999999999");
  });

  test("handsByActions returns only the documented hands payload", () => {
    const store = openStore();

    const result = store.handsByActions({
      ...baseDimension(),
      concreteLineId: 1,
      actions: [],
    });

    expect(result.code).toBe(0);
    expect(result.message).toBeNull();
    expect(Array.isArray(result.data.holeCards)).toBe(true);
    expect(result.data.holeCards.length).toBeGreaterThan(0);
    expect(result.data.exists).toBeUndefined();
  });
});
